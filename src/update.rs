//! The update check: asks GitHub for Marky's latest release tag and compares it with this build.
//!
//! There is no HTTP client in the program: WinHTTP is part of Windows, resolves through the user's
//! proxy settings, and costs no dependency. Only `releases/latest` is read — a request made when the
//! user asks, nothing sent about the document or the machine.

use std::ffi::c_void;

use windows_sys::Win32::Networking::WinHttp::{
    WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest, WinHttpQueryHeaders, WinHttpReadData,
    WinHttpReceiveResponse, WinHttpSendRequest, WinHttpSetTimeouts, INTERNET_DEFAULT_HTTPS_PORT,
    WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_FLAG_SECURE, WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_STATUS_CODE,
};

/// The release page a newer version is downloaded from.
pub const RELEASES_URL: &str = "https://github.com/vikasdfghjl/marky/releases";

const HOST: &str = "api.github.com";
const PATH: &str = "/repos/vikasdfghjl/marky/releases/latest";

/// What the check found, kept for the About panel to show.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Check {
    Failed(String),
    /// The newest release, and whether it is this build or something newer.
    Ready { latest: String, newer: bool },
}

/// Asks GitHub for the latest release tag. Blocking, with timeouts, so the caller runs it off the
/// main thread. The answer's `tag_name` is all that is needed, and it is the newest published,
/// non-draft, non-prerelease release.
pub fn latest() -> Result<String, String> {
    let body = get(HOST, PATH)?;
    let json: serde_json::Value = serde_json::from_str(&body).map_err(|err| format!("unreadable answer: {err}"))?;
    let tag = json.get("tag_name").and_then(serde_json::Value::as_str).ok_or("no tag_name in the answer")?;
    Ok(tag.to_string())
}

/// Whether `latest` is ahead of `current`, comparing numeric versions rather than text, so
/// `0.10.0` beats `0.9.0` and a leading `v` does not matter.
pub fn is_newer(latest: &str, current: &str) -> bool {
    let latest = numbers(latest);
    let current = numbers(current);
    for index in 0..latest.len().max(current.len()) {
        let newer = latest.get(index).copied().unwrap_or(0);
        let older = current.get(index).copied().unwrap_or(0);
        if newer != older {
            return newer > older;
        }
    }
    false
}

/// The numeric parts of a version, ignoring a `v` prefix and anything after the first `-` or `+`.
fn numbers(version: &str) -> Vec<u64> {
    let core = version.trim().trim_start_matches(['v', 'V']);
    let core = core.split(['-', '+']).next().unwrap_or_default();
    core.split('.').map(|part| part.parse().unwrap_or(0)).collect()
}

/// An HTTPS GET, through WinHTTP so the user's proxy and certificates are used.
fn get(host: &str, path: &str) -> Result<String, String> {
    let agent = wide(&format!("Marky/{}", env!("CARGO_PKG_VERSION")));
    let host_w = wide(host);
    let path_w = wide(path);
    unsafe {
        let session = WinHttpOpen(
            agent.as_ptr(),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            std::ptr::null(),
            std::ptr::null(),
            0,
        );
        if session.is_null() {
            return Err("could not start an HTTP session".into());
        }
        let _session = Handle(session);
        WinHttpSetTimeouts(session, 5_000, 5_000, 5_000, 10_000);
        let connection = WinHttpConnect(session, host_w.as_ptr(), INTERNET_DEFAULT_HTTPS_PORT, 0);
        if connection.is_null() {
            return Err("could not reach github.com".into());
        }
        let _connection = Handle(connection);
        let request = WinHttpOpenRequest(
            connection,
            wide("GET").as_ptr(),
            path_w.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            WINHTTP_FLAG_SECURE,
        );
        if request.is_null() {
            return Err("could not build the request".into());
        }
        let _request = Handle(request);
        if WinHttpSendRequest(request, std::ptr::null(), 0, std::ptr::null(), 0, 0, 0) == 0 {
            return Err("the request could not be sent".into());
        }
        if WinHttpReceiveResponse(request, std::ptr::null_mut()) == 0 {
            return Err("no answer from github.com".into());
        }
        let status = status_code(request)?;
        if status != 200 {
            return Err(format!("GitHub answered {status}"));
        }
        read_body(request)
    }
}

unsafe fn status_code(request: *mut c_void) -> Result<u32, String> {
    let mut status: u32 = 0;
    let mut size = std::mem::size_of::<u32>() as u32;
    let ok = WinHttpQueryHeaders(
        request,
        WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
        std::ptr::null(),
        (&raw mut status).cast(),
        &mut size,
        std::ptr::null_mut(),
    );
    if ok == 0 {
        return Err("could not read the answer's status".into());
    }
    Ok(status)
}

unsafe fn read_body(request: *mut c_void) -> Result<String, String> {
    let mut body = Vec::new();
    let mut chunk = [0u8; 8 * 1024];
    loop {
        let mut read: u32 = 0;
        if WinHttpReadData(request, chunk.as_mut_ptr().cast(), chunk.len() as u32, &mut read) == 0 {
            return Err("the answer stopped early".into());
        }
        if read == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..read as usize]);
        // A release's metadata is a few kilobytes; anything this large is not the answer.
        if body.len() > 1 << 20 {
            return Err("the answer was too large".into());
        }
    }
    String::from_utf8(body).map_err(|_| "the answer was not UTF-8".into())
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Closes a WinHTTP handle however the request ends, so no path leaks one.
struct Handle(*mut c_void);

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { WinHttpCloseHandle(self.0) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_by_number_not_text() {
        assert!(is_newer("0.10.0", "0.9.0"));
        assert!(is_newer("v0.4.9", "0.4.8"));
        assert!(is_newer("1.0.0", "0.99.99"));
        assert!(!is_newer("0.4.8", "0.4.8"));
        assert!(!is_newer("0.4.7", "0.4.8"));
        assert!(!is_newer("0.4", "0.4.0"));
        assert!(is_newer("0.4.0.1", "0.4"));
    }

    /// A tag such as `v0.5.0-beta.1` still compares by its release number.
    #[test]
    fn a_suffix_does_not_matter() {
        assert_eq!(numbers("v0.5.0-beta.1"), vec![0, 5, 0]);
        assert_eq!(numbers("0.5.0+build.7"), vec![0, 5, 0]);
        assert!(is_newer("0.5.0-beta.1", "0.4.8"));
    }

    #[test]
    fn the_current_build_is_read_from_cargo() {
        assert!(!is_newer(env!("CARGO_PKG_VERSION"), env!("CARGO_PKG_VERSION")));
    }

    /// The real request, so the WinHTTP call is exercised against GitHub. Ignored by default, since
    /// it needs the network: `cargo test -- --ignored talks_to_github`.
    #[test]
    #[ignore = "needs the network"]
    fn talks_to_github() {
        let tag = latest().expect("GitHub answered with a release tag");
        assert!(tag.starts_with('v') || tag.starts_with(|c: char| c.is_ascii_digit()), "unexpected tag {tag}");
    }
}
