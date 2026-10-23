//! The icon of the program behind a port, for its row in the menu: Node's
//! hexagon, Python's snakes, Spotify's circle. Recognising a program by its
//! icon is quicker than reading its name.
//!
//! Windows asks the shell for the executable's small icon and reads its pixels
//! out. macOS asks NSWorkspace for the icon of the app bundle the executable
//! lives in; a bare Unix binary has only the generic one, so there is none.
//!
//! Dev servers rarely have an icon of their own (`node` is just `node`): the
//! menu tries `tech` and its logos first, and comes here for desktop apps.

pub use imp::load;

#[cfg(windows)]
mod imp {
    use crate::util::{pcwstr, wide};
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits, GetObjectW, BITMAP, BITMAPINFO,
        BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HGDIOBJ,
    };
    use windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES;
    use windows::Win32::UI::Shell::{SHGetFileInfoW, SHFILEINFOW, SHGFI_ICON, SHGFI_SMALLICON};
    use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, ICONINFO};

    /// The executable's small icon as straight-alpha RGBA, with its edge.
    pub fn load(exe: &str) -> Option<(Vec<u8>, u32)> {
        if exe.is_empty() {
            return None;
        }
        let path = wide(exe);
        let mut info = SHFILEINFOW::default();
        let ok = unsafe {
            SHGetFileInfoW(
                pcwstr(&path),
                FILE_FLAGS_AND_ATTRIBUTES(0),
                Some(&mut info),
                std::mem::size_of::<SHFILEINFOW>() as u32,
                SHGFI_ICON | SHGFI_SMALLICON,
            )
        };
        if ok == 0 || info.hIcon.is_invalid() {
            return None;
        }
        let pixels = unsafe { rgba(info.hIcon) };
        unsafe {
            let _ = DestroyIcon(info.hIcon);
        }
        pixels
    }

    unsafe fn rgba(icon: windows::Win32::UI::WindowsAndMessaging::HICON) -> Option<(Vec<u8>, u32)> {
        let mut ii = ICONINFO::default();
        GetIconInfo(icon, &mut ii).ok()?;
        // Both bitmaps are ours to free once GetIconInfo succeeds.
        let free = |ii: &ICONINFO| {
            let _ = DeleteObject(HGDIOBJ(ii.hbmColor.0));
            let _ = DeleteObject(HGDIOBJ(ii.hbmMask.0));
        };
        if ii.hbmColor.is_invalid() {
            free(&ii);
            return None;
        }
        let mut bm = BITMAP::default();
        if GetObjectW(
            HGDIOBJ(ii.hbmColor.0),
            std::mem::size_of::<BITMAP>() as i32,
            Some((&mut bm as *mut BITMAP).cast()),
        ) == 0
            || bm.bmWidth <= 0
            || bm.bmWidth != bm.bmHeight
        {
            free(&ii);
            return None;
        }
        let edge = bm.bmWidth as u32;
        let mut bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: bm.bmWidth,
                // Negative: top-down rows, the order the menu expects.
                biHeight: -bm.bmHeight,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut px = vec![0u8; (edge * edge * 4) as usize];
        let dc = CreateCompatibleDC(None);
        let lines = GetDIBits(
            dc,
            ii.hbmColor,
            0,
            edge,
            Some(px.as_mut_ptr().cast()),
            &mut bmi,
            DIB_RGB_COLORS,
        );
        let _ = DeleteDC(dc);
        free(&ii);
        if lines == 0 {
            return None;
        }
        // BGRA → RGBA. Icons from before alpha channels have none at all; for
        // those, any coloured pixel is opaque.
        let has_alpha = px.chunks(4).any(|p| p[3] != 0);
        for p in px.chunks_mut(4) {
            p.swap(0, 2);
            if !has_alpha {
                p[3] = if p[0] | p[1] | p[2] != 0 { 255 } else { 0 };
            }
        }
        Some((px, edge))
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use objc2::AnyThread;
    use objc2_app_kit::{
        NSBitmapFormat, NSBitmapImageRep, NSCompositingOperation, NSDeviceRGBColorSpace,
        NSGraphicsContext, NSWorkspace,
    };
    use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

    /// Pixels on a side: menus draw icons at 16 pt, and this is 16 pt @2x.
    const EDGE: u32 = 32;

    /// The icon of the app bundle around `exe`, as straight-alpha RGBA.
    pub fn load(exe: &str) -> Option<(Vec<u8>, u32)> {
        let bundle = app_bundle(exe)?;
        let icon = NSWorkspace::sharedWorkspace().iconForFile(&NSString::from_str(bundle));
        let rep = unsafe {
            NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bitmapFormat_bytesPerRow_bitsPerPixel(
                NSBitmapImageRep::alloc(),
                std::ptr::null_mut(),
                EDGE as isize,
                EDGE as isize,
                8,
                4,
                true,
                false,
                NSDeviceRGBColorSpace,
                // Premultiplied: a bitmap context can't draw into anything else.
                NSBitmapFormat::empty(),
                (EDGE * 4) as isize,
                32,
            )
        }?;
        let ctx = NSGraphicsContext::graphicsContextWithBitmapImageRep(&rep)?;
        let rect = NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(EDGE as f64, EDGE as f64),
        );
        NSGraphicsContext::saveGraphicsState_class();
        NSGraphicsContext::setCurrentContext(Some(&ctx));
        icon.drawInRect_fromRect_operation_fraction(
            rect,
            NSRect::ZERO,
            NSCompositingOperation::SourceOver,
            1.0,
        );
        ctx.flushGraphics();
        NSGraphicsContext::restoreGraphicsState_class();

        let data = rep.bitmapData();
        if data.is_null() {
            return None;
        }
        let stride = rep.bytesPerRow() as usize;
        let row = (EDGE * 4) as usize;
        let mut px = Vec::with_capacity(row * EDGE as usize);
        for y in 0..EDGE as usize {
            px.extend_from_slice(unsafe { std::slice::from_raw_parts(data.add(y * stride), row) });
        }
        // Back to straight alpha, which is what menus take.
        for p in px.chunks_mut(4) {
            let a = p[3] as u32;
            if a != 0 && a != 255 {
                for c in &mut p[..3] {
                    *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
                }
            }
        }
        Some((px, EDGE))
    }

    /// "/Applications/Docker.app/Contents/MacOS/com.docker.backend" →
    /// "/Applications/Docker.app". The outermost bundle, so a helper app
    /// nested inside another shows the app people know.
    pub(super) fn app_bundle(exe: &str) -> Option<&str> {
        let end = exe.find(".app/")? + ".app".len();
        Some(&exe[..end])
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    #[test]
    fn bundles() {
        use super::imp::app_bundle;
        assert_eq!(
            app_bundle("/Applications/Visual Studio Code.app/Contents/Frameworks/Code Helper (Plugin).app/Contents/MacOS/Code Helper (Plugin)"),
            Some("/Applications/Visual Studio Code.app")
        );
        assert_eq!(app_bundle("/opt/homebrew/bin/node"), None);
    }

    #[test]
    fn finder_has_an_icon() {
        let (px, edge) =
            super::load("/System/Library/CoreServices/Finder.app/Contents/MacOS/Finder")
                .expect("an icon");
        assert_eq!(px.len(), (edge * edge * 4) as usize);
        assert!(px.chunks(4).any(|p| p[3] == 255), "drawn, not blank");
    }
}

#[cfg(all(test, windows))]
mod tests {
    /// By hand: `cargo test icons_of_real_programs -- --ignored --nocapture`
    /// writes the icons of a few programs to target/icons for a look.
    #[test]
    #[ignore]
    fn icons_of_real_programs() {
        std::fs::create_dir_all("target/icons").unwrap();
        for exe in std::env::var("PORTA_ICON_EXES")
            .unwrap_or_default()
            .split(';')
        {
            match super::load(exe) {
                Some((px, edge)) => {
                    let name = exe.rsplit(['\\', '/']).next().unwrap();
                    std::fs::write(format!("target/icons/prog-{name}-{edge}.rgba"), px).unwrap();
                    println!("{exe}: {edge}px");
                }
                None => println!("{exe}: no icon"),
            }
        }
    }
}
