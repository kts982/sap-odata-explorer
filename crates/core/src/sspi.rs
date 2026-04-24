//! Windows SSPI/SPNEGO module for Kerberos SSO authentication.
//!
//! Uses the Windows Security Support Provider Interface (SSPI) to generate
//! SPNEGO/Negotiate tokens from the current Windows domain session.

#[cfg(windows)]
mod imp {
    use base64::Engine;
    use tracing::debug;
    use windows::Win32::Security::Authentication::Identity::{
        AcquireCredentialsHandleW, DeleteSecurityContext, FreeContextBuffer, FreeCredentialsHandle,
        ISC_REQ_ALLOCATE_MEMORY, ISC_REQ_DELEGATE, ISC_REQ_MUTUAL_AUTH, InitializeSecurityContextW,
        SECBUFFER_TOKEN, SECBUFFER_VERSION, SECPKG_CRED_OUTBOUND, SecBuffer, SecBufferDesc,
    };
    use windows::Win32::Security::Credentials::SecHandle;

    /// Generate a SPNEGO/Negotiate token for the given SAP host.
    /// Returns a base64-encoded token for the `Authorization: Negotiate <token>` header.
    pub fn generate_negotiate_token(target_host: &str) -> Result<String, String> {
        unsafe { generate_token_unsafe(target_host) }
    }

    unsafe fn generate_token_unsafe(target_host: &str) -> Result<String, String> {
        let spn = format!("HTTP/{}", target_host);
        let spn_wide: Vec<u16> = spn.encode_utf16().chain(std::iter::once(0)).collect();

        let package = "Negotiate";
        let package_wide: Vec<u16> = package.encode_utf16().chain(std::iter::once(0)).collect();

        // Step 1: Acquire credentials handle for current Windows session
        let mut cred_handle = SecHandle::default();
        let mut expiry: i64 = 0;

        let result = unsafe {
            AcquireCredentialsHandleW(
                None,
                windows::core::PCWSTR(package_wide.as_ptr()),
                SECPKG_CRED_OUTBOUND,
                None,
                None,
                None,
                None,
                &mut cred_handle as *mut _,
                Some(&mut expiry as *mut _),
            )
        };

        if let Err(e) = result {
            return Err(format!(
                "AcquireCredentialsHandle failed: {}. Is this machine domain-joined?",
                e
            ));
        }

        debug!("SSPI: credentials acquired for current user");

        // Step 2: Initialize security context to get the SPNEGO token
        let mut out_buffer = SecBuffer {
            cbBuffer: 0,
            BufferType: SECBUFFER_TOKEN,
            pvBuffer: std::ptr::null_mut(),
        };
        let mut out_desc = SecBufferDesc {
            ulVersion: SECBUFFER_VERSION,
            cBuffers: 1,
            pBuffers: &mut out_buffer as *mut _,
        };

        let mut context_handle = SecHandle::default();
        let mut context_attrs: u32 = 0;
        let mut context_expiry: i64 = 0;

        let isc_flags = ISC_REQ_DELEGATE | ISC_REQ_MUTUAL_AUTH | ISC_REQ_ALLOCATE_MEMORY;

        let hresult = unsafe {
            InitializeSecurityContextW(
                Some(&cred_handle as *const _),
                None,
                Some(spn_wide.as_ptr()),
                isc_flags,
                0,
                0,
                None,
                0,
                Some(&mut context_handle as *mut _),
                Some(&mut out_desc as *mut _),
                &mut context_attrs as *mut _,
                Some(&mut context_expiry as *mut _),
            )
        };

        // Clean up credentials
        let _ = unsafe { FreeCredentialsHandle(&cred_handle as *const _) };

        // Check result: SEC_E_OK (0) or SEC_I_CONTINUE_NEEDED (0x00090312)
        if hresult.is_err() {
            return Err(format!(
                "InitializeSecurityContext failed: 0x{:08x}. \
                 SPN '{}' may not be registered, or domain trust not set up.",
                hresult.0 as u32, spn
            ));
        }

        if out_buffer.pvBuffer.is_null() || out_buffer.cbBuffer == 0 {
            return Err("SSPI returned empty token".to_string());
        }

        let token_bytes = unsafe {
            std::slice::from_raw_parts(
                out_buffer.pvBuffer as *const u8,
                out_buffer.cbBuffer as usize,
            )
        };

        let token_b64 = base64::engine::general_purpose::STANDARD.encode(token_bytes);
        debug!(
            "SSPI: Negotiate token generated ({} bytes)",
            out_buffer.cbBuffer
        );

        // Free SSPI resources
        unsafe {
            let _ = FreeContextBuffer(out_buffer.pvBuffer);
            let _ = DeleteSecurityContext(&context_handle as *const _);
        }

        Ok(token_b64)
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn generate_negotiate_token(_target_host: &str) -> Result<String, String> {
        Err("SSO/SPNEGO is only available on Windows".to_string())
    }
}

pub use imp::generate_negotiate_token;
