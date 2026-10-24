//! A feed's credentials as NuGet keeps them: in memory, printed nowhere, and a stored password as NuGet stores it, in
//! clear text, or on Windows encrypted for the user (DPAPI, NuGet's entropy, base64).

/// A user name and password for a feed. Its `Debug` names the user and never the password, so a log line or a value
/// printed in a test leaks nothing.
#[derive(Clone, PartialEq, Eq)]
pub struct Credentials {
    pub username: String,
    pub password: String,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "credentials of {}", self.username)
    }
}

impl Credentials {
    /// The value of an HTTP Basic Authorization header.
    pub fn basic(&self) -> String {
        format!("Basic {}", encode(format!("{}:{}", self.username, self.password).as_bytes()))
    }
}

/// A `Password` NuGet encrypted for the user: readable on Windows alone, as NuGet reads it.
pub fn unprotect(stored: &str) -> Option<String> {
    #[cfg(windows)]
    {
        String::from_utf8(dpapi::unprotect(&decode(stored.trim())?)?).ok()
    }
    #[cfg(not(windows))]
    {
        let _ = stored;
        None
    }
}

/// A password as `dotnet nuget` stores it: on Windows encrypted (`Password`), else in clear text
/// (`ClearTextPassword`). The key and the value to write.
pub fn protect(password: &str) -> (&'static str, String) {
    #[cfg(windows)]
    if let Some(sealed) = dpapi::protect(password.as_bytes()) {
        return ("Password", encode(&sealed));
    }
    ("ClearTextPassword", password.to_string())
}

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = u32::from(chunk[0]) << 16
            | u32::from(*chunk.get(1).unwrap_or(&0)) << 8
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

pub fn decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let (mut bits, mut held) = (0u32, 0);
    for c in text.bytes().filter(|c| !c.is_ascii_whitespace()) {
        if c == b'=' {
            break;
        }
        bits = bits << 6 | ALPHABET.iter().position(|a| *a == c)? as u32;
        held += 6;
        if held >= 8 {
            held -= 8;
            out.push((bits >> held) as u8);
        }
    }
    Some(out)
}

#[cfg(windows)]
mod dpapi {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
    };

    /// NuGet's own entropy, so what NuGet sealed opens here and nowhere without it.
    const ENTROPY: &[u8] = b"NuGet";

    fn blob(bytes: &[u8]) -> CRYPT_INTEGER_BLOB {
        CRYPT_INTEGER_BLOB { cbData: bytes.len() as u32, pbData: bytes.as_ptr() as *mut u8 }
    }

    /// DPAPI's output copied out, then freed as DPAPI asks.
    unsafe fn taken(out: CRYPT_INTEGER_BLOB) -> Vec<u8> {
        // SAFETY: DPAPI gave a buffer of cbData bytes, which LocalFree releases.
        unsafe {
            let bytes = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
            LocalFree(out.pbData as _);
            bytes
        }
    }

    pub fn protect(clear: &[u8]) -> Option<Vec<u8>> {
        let (data, entropy) = (blob(clear), blob(ENTROPY));
        let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
        // SAFETY: every pointer is to a live local for the call's length.
        let done = unsafe {
            CryptProtectData(
                &data,
                std::ptr::null(),
                &entropy,
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
        };
        // SAFETY: on success `out` is DPAPI's own buffer.
        (done != 0).then(|| unsafe { taken(out) })
    }

    pub fn unprotect(sealed: &[u8]) -> Option<Vec<u8>> {
        let (data, entropy) = (blob(sealed), blob(ENTROPY));
        let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
        // SAFETY: every pointer is to a live local for the call's length; no description is asked for.
        let done = unsafe {
            CryptUnprotectData(
                &data,
                std::ptr::null_mut(),
                &entropy,
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
        };
        // SAFETY: on success `out` is DPAPI's own buffer.
        (done != 0).then(|| unsafe { taken(out) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trips_and_matches_the_basic_header_rfc_7617_gives() {
        let basic = Credentials { username: "Aladdin".into(), password: "open sesame".into() }.basic();
        assert_eq!(basic, "Basic QWxhZGRpbjpvcGVuIHNlc2FtZQ==");
        for bytes in [&b""[..], b"a", b"ab", b"abc", b"abcd", &[0, 255, 128, 7, 9]] {
            assert_eq!(decode(&encode(bytes)).unwrap(), bytes);
        }
        assert_eq!(decode("not*base64"), None);
    }

    #[test]
    fn credentials_print_the_user_and_never_the_password() {
        let printed = format!("{:?}", Credentials { username: "ann".into(), password: "hunter2".into() });
        assert_eq!(printed, "credentials of ann");
    }

    #[test]
    fn a_stored_password_reads_back_as_dotnet_nuget_stores_it() {
        let (key, value) = protect("s3cret");
        if cfg!(windows) {
            assert_eq!(key, "Password");
            assert_eq!(unprotect(&value).as_deref(), Some("s3cret"), "a password sealed for the user did not open");
        } else {
            assert_eq!((key, value.as_str()), ("ClearTextPassword", "s3cret"));
            assert_eq!(
                unprotect("AQAAANCMnd8BFdER"),
                None,
                "an encrypted password was read where DPAPI does not exist"
            );
        }
    }
}
