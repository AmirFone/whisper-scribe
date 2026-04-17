use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

static PERMISSION_LOGGED: AtomicBool = AtomicBool::new(false);

#[cfg(target_os = "macos")]
mod cg {
    use std::ffi::c_void;
    use std::path::Path;

    #[repr(C)]
    pub struct CGImage(c_void);
    type CFStringRef = *const c_void;
    type CFURLRef = *const c_void;
    type CFDictionaryRef = *const c_void;
    type CGImageDestinationRef = *const c_void;
    type CGImageRef = *const CGImage;

    unsafe extern "C" {
        pub fn CGPreflightScreenCaptureAccess() -> bool;
        pub fn CGRequestScreenCaptureAccess() -> bool;
        fn CGMainDisplayID() -> u32;
        fn CGGetOnlineDisplayList(max: u32, displays: *mut u32, count: *mut u32) -> i32;
        fn CGDisplayCreateImage(display_id: u32) -> CGImageRef;
        fn CGImageGetWidth(image: CGImageRef) -> usize;
        fn CGImageRelease(image: CGImageRef);

        // ImageIO — write CGImage to file as PNG
        fn CGImageDestinationCreateWithURL(
            url: CFURLRef,
            image_type: CFStringRef,
            count: usize,
            options: CFDictionaryRef,
        ) -> CGImageDestinationRef;
        fn CGImageDestinationAddImage(
            dest: CGImageDestinationRef,
            image: CGImageRef,
            properties: CFDictionaryRef,
        );
        fn CGImageDestinationFinalize(dest: CGImageDestinationRef) -> bool;

        // CoreFoundation helpers
        fn CFRelease(cf: *const c_void);
        fn CFURLCreateWithFileSystemPath(
            allocator: *const c_void,
            path: CFStringRef,
            style: i32,
            is_dir: bool,
        ) -> CFURLRef;
        fn CFStringCreateWithBytes(
            allocator: *const c_void,
            bytes: *const u8,
            len: i64,
            encoding: u32,
            is_external: bool,
        ) -> CFStringRef;
    }

    const K_CF_STRING_ENCODING_UTF8: u32 = 0x08000100;
    const K_CF_URL_POSIX_PATH_STYLE: i32 = 0;

    fn cfstring(s: &str) -> CFStringRef {
        unsafe {
            CFStringCreateWithBytes(
                std::ptr::null(),
                s.as_ptr(),
                s.len() as i64,
                K_CF_STRING_ENCODING_UTF8,
                false,
            )
        }
    }

    pub fn preflight() -> bool {
        unsafe { CGPreflightScreenCaptureAccess() }
    }

    pub fn request() -> bool {
        unsafe { CGRequestScreenCaptureAccess() }
    }

    pub fn get_display_ids() -> Vec<u32> {
        let mut ids = [0u32; 16];
        let mut count: u32 = 0;
        let err = unsafe { CGGetOnlineDisplayList(16, ids.as_mut_ptr(), &mut count) };
        if err != 0 || count == 0 {
            return vec![unsafe { CGMainDisplayID() }];
        }
        ids[..count as usize].to_vec()
    }

    pub fn capture_display(display_id: u32) -> Option<OwnedCGImage> {
        let img = unsafe { CGDisplayCreateImage(display_id) };
        if img.is_null() {
            return None;
        }
        let width = unsafe { CGImageGetWidth(img) };
        if width == 0 {
            unsafe { CGImageRelease(img) };
            return None;
        }
        Some(OwnedCGImage(img))
    }

    pub struct OwnedCGImage(CGImageRef);

    impl OwnedCGImage {
        pub fn save_png(&self, path: &Path) -> bool {
            let path_str = match path.to_str() {
                Some(s) => s,
                None => return false,
            };

            unsafe {
                let cf_path = cfstring(path_str);
                if cf_path.is_null() {
                    return false;
                }
                let url = CFURLCreateWithFileSystemPath(
                    std::ptr::null(),
                    cf_path,
                    K_CF_URL_POSIX_PATH_STYLE,
                    false,
                );
                CFRelease(cf_path);
                if url.is_null() {
                    return false;
                }

                let png_type = cfstring("public.png");
                let dest = CGImageDestinationCreateWithURL(url, png_type, 1, std::ptr::null());
                CFRelease(url);
                CFRelease(png_type);
                if dest.is_null() {
                    return false;
                }

                CGImageDestinationAddImage(dest, self.0, std::ptr::null());
                let ok = CGImageDestinationFinalize(dest);
                CFRelease(dest);
                ok
            }
        }
    }

    impl Drop for OwnedCGImage {
        fn drop(&mut self) {
            unsafe { CGImageRelease(self.0) };
        }
    }
}

#[cfg(target_os = "macos")]
pub fn has_screen_capture_permission() -> bool {
    cg::preflight()
}

#[cfg(not(target_os = "macos"))]
pub fn has_screen_capture_permission() -> bool {
    false
}

#[cfg(target_os = "macos")]
pub fn request_screen_capture_permission() -> bool {
    cg::request()
}

#[cfg(target_os = "macos")]
pub fn capture_all_screens(output_dir: &Path) -> Result<Vec<PathBuf>, String> {
    if !has_screen_capture_permission() {
        if !PERMISSION_LOGGED.swap(true, Ordering::Relaxed) {
            log::warn!("Screen recording permission not granted — skipping capture. Grant access in System Settings > Privacy & Security > Screen Recording.");
        }
        return Err("Screen recording permission not granted".to_string());
    }

    std::fs::create_dir_all(output_dir)
        .map_err(|e| format!("Failed to create screenshots dir: {e}"))?;

    let display_ids = cg::get_display_ids();
    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
    let mut paths = Vec::new();

    for (i, &display_id) in display_ids.iter().enumerate() {
        let path = output_dir.join(format!("screen_{}_{timestamp}.png", i + 1));
        match cg::capture_display(display_id) {
            Some(image) => {
                if image.save_png(&path) && path.exists() {
                    paths.push(path);
                } else {
                    log::warn!("Failed to save screenshot for display {}", i + 1);
                }
            }
            None => {
                log::warn!("CGDisplayCreateImage returned null for display {display_id}");
            }
        }
    }

    if paths.is_empty() {
        return Err("No screenshots captured".to_string());
    }

    log::info!("Captured {} screenshot(s)", paths.len());
    Ok(paths)
}

#[cfg(not(target_os = "macos"))]
pub fn capture_all_screens(_output_dir: &Path) -> Result<Vec<PathBuf>, String> {
    log::warn!("Screen capture is only supported on macOS");
    Ok(Vec::new())
}

pub fn cleanup_old_screenshots(dir: &Path, max_age: Duration) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    let cutoff = std::time::SystemTime::now() - max_age;

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("png") {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        let Ok(modified) = meta.modified() else { continue };
        if modified < cutoff {
            log::info!("Cleaning up old screenshot: {}", path.display());
            std::fs::remove_file(&path).ok();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_cleanup_deletes_old_pngs() {
        // #given a temp dir with an "old" png and a "new" png
        let dir = TempDir::new().unwrap();
        let old_path = dir.path().join("screen_old.png");
        let new_path = dir.path().join("screen_new.png");
        fs::write(&old_path, b"old screenshot").unwrap();
        fs::write(&new_path, b"new screenshot").unwrap();

        let two_hours_ago = std::time::SystemTime::now() - Duration::from_secs(7200);
        filetime::set_file_mtime(
            &old_path,
            filetime::FileTime::from_system_time(two_hours_ago),
        )
        .unwrap();

        // #when we cleanup with max_age = 1 hour
        cleanup_old_screenshots(dir.path(), Duration::from_secs(3600));

        // #then old file is deleted, new file remains
        assert!(!old_path.exists(), "old screenshot should be deleted");
        assert!(new_path.exists(), "new screenshot should remain");
    }

    #[test]
    fn test_cleanup_ignores_non_png_files() {
        let dir = TempDir::new().unwrap();
        let txt_path = dir.path().join("notes.txt");
        fs::write(&txt_path, b"not a screenshot").unwrap();

        let two_hours_ago = std::time::SystemTime::now() - Duration::from_secs(7200);
        filetime::set_file_mtime(
            &txt_path,
            filetime::FileTime::from_system_time(two_hours_ago),
        )
        .unwrap();

        cleanup_old_screenshots(dir.path(), Duration::from_secs(3600));
        assert!(txt_path.exists());
    }

    #[test]
    fn test_cleanup_handles_empty_dir() {
        let dir = TempDir::new().unwrap();
        cleanup_old_screenshots(dir.path(), Duration::from_secs(3600));
    }

    #[test]
    fn test_cleanup_handles_nonexistent_dir() {
        cleanup_old_screenshots(Path::new("/nonexistent/dir"), Duration::from_secs(3600));
    }

    #[test]
    fn test_cleanup_keeps_recent_pngs() {
        let dir = TempDir::new().unwrap();
        let path1 = dir.path().join("screen_1.png");
        let path2 = dir.path().join("screen_2.png");
        fs::write(&path1, b"recent 1").unwrap();
        fs::write(&path2, b"recent 2").unwrap();

        cleanup_old_screenshots(dir.path(), Duration::from_secs(3600));
        assert!(path1.exists());
        assert!(path2.exists());
    }
}
