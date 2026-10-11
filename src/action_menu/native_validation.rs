//! Opt-in Windows validation; uses hidden HWNDs and offscreen GDI, never OS input.

use super::*;
use windows::Win32::{
    Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS,
        DeleteDC, GdiFlush,
    },
    UI::WindowsAndMessaging::{
        DispatchMessageW, IsWindow, MSG, PM_REMOVE, PeekMessageW, SendMessageW,
    },
};

fn capture(name: &str, state: &NativeRadialState) -> image::RgbaImage {
    let dc = unsafe { CreateCompatibleDC(None) };
    assert!(!dc.0.is_null());
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: WHEEL_SIZE,
            biHeight: -WHEEL_SIZE,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits = std::ptr::null_mut();
    let bitmap =
        unsafe { CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0) }.unwrap();
    let old = unsafe { select(dc, HGDIOBJ(bitmap.0)) };
    unsafe {
        render_wheel(dc, state);
        let _ = GdiFlush();
    }
    let mut rgba = unsafe {
        std::slice::from_raw_parts(bits.cast::<u8>(), (WHEEL_SIZE * WHEEL_SIZE * 4) as usize)
    }
    .to_vec();
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.swap(0, 2);
        pixel[3] = if pixel[..3] == [KEY_RGB.0, KEY_RGB.1, KEY_RGB.2] {
            0
        } else {
            255
        };
    }
    unsafe {
        let _ = select(dc, old);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let _ = DeleteDC(dc);
    }
    let image = image::RgbaImage::from_raw(WHEEL_SIZE as u32, WHEEL_SIZE as u32, rgba).unwrap();
    std::fs::create_dir_all("target/radial-native").unwrap();
    image
        .save(format!("target/radial-native/{name}.png"))
        .unwrap();
    image
}

fn pump_for(duration: Duration) {
    let end = Instant::now() + duration;
    while Instant::now() < end {
        let mut message = MSG::default();
        while unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) }.as_bool() {
            let _ = unsafe { DispatchMessageW(&message) };
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn click(hwnd: HWND, index: usize, page: RadialPage) {
    let (x, y) = item_position(index, items_for(page).len());
    let packed = LPARAM(((y as u32) << 16 | x as u32) as isize);
    let _ = unsafe { SendMessageW(hwnd, WM_LBUTTONUP, None, Some(packed)) };
}

fn create_hidden_wheel() -> HWND {
    *native_state().lock().unwrap() = NativeRadialState::default();
    let instance = ensure_radial_class().unwrap();
    let hwnd = unsafe {
        CreateWindowExW(
            Default::default(),
            w!("NeraRadialMenuWindowClass"),
            w!("Nera radial validation"),
            WS_POPUP,
            0,
            0,
            WHEEL_SIZE,
            WHEEL_SIZE,
            None,
            None,
            Some(instance),
            None,
        )
    }
    .unwrap();
    RADIAL_HWND.store(hwnd.0 as isize, Ordering::Release);
    hwnd
}

#[test]
#[ignore = "Native Windows radial rendering/timer validation; writes PNGs without mouse/keyboard control"]
fn native_radial_feedback_and_lifecycle() {
    let normal = capture("normal", &NativeRadialState::default());
    assert_eq!(
        normal.get_pixel(WHEEL_CENTER as u32, WHEEL_CENTER as u32)[3],
        0
    );
    let hwnd = create_hidden_wheel();
    // A disabled section stays inert.
    click(hwnd, 4, RadialPage::Root);
    assert!(native_state().lock().unwrap().pending.is_none());

    let started = Instant::now();
    click(hwnd, 0, RadialPage::Root);
    click(hwnd, 1, RadialPage::Root); // Duplicate cannot replace the first choice.
    let selected = *native_state().lock().unwrap();
    assert_eq!(selected.pending.unwrap().index, 0);
    assert_eq!(selected.page, RadialPage::Root);
    let pressed = capture("selected-cloud", &selected);
    assert_eq!(
        &pressed.get_pixel(WHEEL_CENTER as u32, 35).0[..3],
        &[SELECTED_RGB.0, SELECTED_RGB.1, SELECTED_RGB.2]
    );
    let mut hover = NativeRadialState::default();
    hover.hover = Some(6);
    capture("hover-screensaver", &hover);
    // A real Win32 timer, serviced on this test thread, opens the submenu.
    pump_for(Duration::from_millis(220));
    assert!(started.elapsed() >= Duration::from_millis(SELECTION_FEEDBACK_MS.into()));
    let cloud = *native_state().lock().unwrap();
    assert_eq!(cloud.page, RadialPage::Cloud);
    assert!(cloud.pending.is_none());
    assert!(!cloud.result_sent);
    capture("cloud-submenu", &cloud);

    // Back/Escape cancels a pending command rather than dispatching it later.
    click(hwnd, 0, RadialPage::Cloud);
    let _ = unsafe { SendMessageW(hwnd, WM_KEYDOWN, Some(WPARAM(VK_ESCAPE_RAW as usize)), None) };
    pump_for(Duration::from_millis(220));
    let state = *native_state().lock().unwrap();
    assert_eq!(state.page, RadialPage::Root);
    assert!(state.pending.is_none());
    assert!(!state.result_sent);

    // Release coordinates win over stale hover, then dispatch closes the HWND.
    native_state().lock().unwrap().hover = Some(1);
    click(hwnd, 5, RadialPage::Root);
    assert_eq!(native_state().lock().unwrap().pending.unwrap().index, 5);
    assert!(unsafe { IsWindow(Some(hwnd)) }.as_bool());
    pump_for(Duration::from_millis(220));
    assert!(!unsafe { IsWindow(Some(hwnd)) }.as_bool());
    assert!(native_state().lock().unwrap().result_sent);
    assert_eq!(RADIAL_HWND.load(Ordering::Acquire), 0);

    // Focus loss destroys a fresh pending selection immediately, without a late action.
    let hwnd = create_hidden_wheel();
    click(hwnd, 1, RadialPage::Root);
    let _ = unsafe { SendMessageW(hwnd, WM_KILLFOCUS, None, None) };
    pump_for(Duration::from_millis(220));
    let mut state = native_state().lock().unwrap();
    assert!(state.result_sent);
    assert!(state.pending.is_none());
    assert_eq!(state.take_ready_selection(Instant::now()), None);
    assert!(!unsafe { IsWindow(Some(hwnd)) }.as_bool());
    println!(
        "PASSED: native GDI icons/sector feedback; real 150ms timer; submenu; duplicate click; disabled item; stale hover; Escape/focus cancellation; single close. PNGs: target/radial-native/"
    );
}
