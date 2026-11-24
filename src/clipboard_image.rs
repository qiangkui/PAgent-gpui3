//! Reading an image off the clipboard.
//!
//! A screenshot is the most direct way to show the assistant what is wrong, and
//! the only way in was to save the file first. On Windows the clipboard carries
//! a device-independent bitmap, which is turned into PNG bytes an attachment can
//! hold; other platforms report nothing and the paste simply stays text.

/// Largest clipboard bitmap that is read, in bytes.
const MAX_DIB_BYTES: usize = 48 * 1024 * 1024;

/// The clipboard's image as PNG, or `None` when there is none.
#[cfg(windows)]
pub fn png_from_clipboard() -> Option<Vec<u8>> {
    dib_from_clipboard().and_then(|dib| png_from_dib(&dib))
}

/// Nothing to read on a platform without the Windows clipboard layout.
#[cfg(not(windows))]
pub fn png_from_clipboard() -> Option<Vec<u8>> {
    None
}

/// The raw `CF_DIB` payload on the clipboard.
#[cfg(windows)]
fn dib_from_clipboard() -> Option<Vec<u8>> {
    /// `CF_DIB`.
    const CF_DIB: u32 = 8;

    #[link(name = "user32")]
    unsafe extern "system" {
        fn OpenClipboard(owner: isize) -> i32;
        fn CloseClipboard() -> i32;
        fn IsClipboardFormatAvailable(format: u32) -> i32;
        fn GetClipboardData(format: u32) -> isize;
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GlobalLock(handle: isize) -> *mut u8;
        fn GlobalUnlock(handle: isize) -> i32;
        fn GlobalSize(handle: isize) -> usize;
    }

    // SAFETY: the clipboard is opened once and closed on every path; the handle
    // is only read while locked, and the copy is made before unlocking.
    unsafe {
        if OpenClipboard(0) == 0 {
            return None;
        }
        let answer = if IsClipboardFormatAvailable(CF_DIB) == 0 {
            None
        } else {
            let handle = GetClipboardData(CF_DIB);
            if handle == 0 {
                None
            } else {
                let size = GlobalSize(handle);
                let pointer = GlobalLock(handle);
                if size == 0 || size > MAX_DIB_BYTES || pointer.is_null() {
                    None
                } else {
                    let bytes = std::slice::from_raw_parts(pointer, size).to_vec();
                    let _ = GlobalUnlock(handle);
                    Some(bytes)
                }
            }
        };
        let _ = CloseClipboard();
        answer
    }
}

/// Turn a clipboard DIB into PNG bytes.
///
/// `CF_DIB` is a `BITMAPINFOHEADER` and its pixels without the 14-byte file
/// header a `.bmp` starts with, so the header is put back and the bitmap decoded
/// from there; a bitmap that cannot be decoded is reported as "no image".
fn png_from_dib(dib: &[u8]) -> Option<Vec<u8>> {
    if dib.len() < 40 {
        return None;
    }
    let header_size = u32::from_le_bytes(dib[0..4].try_into().ok()?) as usize;
    if header_size < 40 || header_size > dib.len() {
        return None;
    }
    let mut bmp = Vec::with_capacity(14 + dib.len());
    let file_size = (14 + dib.len()) as u32;
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&file_size.to_le_bytes());
    bmp.extend_from_slice(&0u32.to_le_bytes());
    bmp.extend_from_slice(&(14 + header_size as u32).to_le_bytes());
    bmp.extend_from_slice(dib);
    let image = image::load_from_memory_with_format(&bmp, image::ImageFormat::Bmp).ok()?;
    let mut png = std::io::Cursor::new(Vec::new());
    image.write_to(&mut png, image::ImageFormat::Png).ok()?;
    Some(png.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 1×1 24-bit bottom-up DIB: header, then one padded pixel row.
    fn one_pixel_dib() -> Vec<u8> {
        let mut dib = Vec::new();
        dib.extend_from_slice(&40u32.to_le_bytes()); // biSize
        dib.extend_from_slice(&1i32.to_le_bytes()); // biWidth
        dib.extend_from_slice(&1i32.to_le_bytes()); // biHeight
        dib.extend_from_slice(&1u16.to_le_bytes()); // biPlanes
        dib.extend_from_slice(&24u16.to_le_bytes()); // biBitCount
        dib.extend_from_slice(&0u32.to_le_bytes()); // biCompression
        dib.extend_from_slice(&4u32.to_le_bytes()); // biSizeImage
        dib.extend_from_slice(&[0u8; 16]); // DPI and palette counts
        dib.extend_from_slice(&[0u8, 0u8, 0xffu8, 0u8]); // red pixel + padding
        dib
    }

    #[test]
    fn a_dib_becomes_a_png_an_attachment_can_carry() {
        let png = png_from_dib(&one_pixel_dib()).expect("png");
        assert!(png.starts_with(&[0x89, b'P', b'N', b'G']), "PNG signature");
    }

    #[test]
    fn a_truncated_or_empty_bitmap_reports_nothing() {
        assert!(png_from_dib(&[]).is_none());
        assert!(png_from_dib(&[0u8; 12]).is_none());
        // A header that claims to be smaller than a bitmap header is not one.
        let mut broken = one_pixel_dib();
        broken[0..4].copy_from_slice(&4u32.to_le_bytes());
        assert!(png_from_dib(&broken).is_none());
    }
}