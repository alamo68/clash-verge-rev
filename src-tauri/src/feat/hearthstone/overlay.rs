use super::scene::GameSceneMonitor;
use crate::{
    config::{Config, IVerge},
    process::AsyncHandler,
};
use anyhow::{Context as _, Result};
use clash_verge_draft::Draft;
use clash_verge_logging::{Type, logging};
use objc2::{DefinedClass as _, MainThreadOnly, define_class, msg_send, rc::Retained, runtime::AnyObject, sel};
use objc2_app_kit::{
    NSAttributedStringNSStringDrawing as _, NSAutoresizingMaskOptions, NSBackingStoreType, NSBezierPath, NSColor,
    NSEvent, NSFloatingWindowLevel, NSFont, NSFontAttributeName, NSFontWeightSemibold, NSForegroundColorAttributeName,
    NSPanel, NSRunningApplication, NSScreen, NSView, NSWindow, NSWindowCollectionBehavior, NSWindowStyleMask,
    NSWorkspace,
};
use objc2_foundation::{
    MainThreadMarker, NSArray, NSAttributedString, NSDictionary, NSNumber, NSPoint, NSRect, NSRunLoop,
    NSRunLoopCommonModes, NSSize, NSString, NSTimer,
};
use std::{
    cell::{Cell, RefCell},
    time::{Duration, Instant},
};
use tauri::AppHandle;

const GAME_BUNDLE: &str = "unity.Blizzard Entertainment.Hearthstone";
const TRACKER_BUNDLE: &str = "net.hearthsim.hstracker";
const BUTTON_TITLE: &str = "一键拔线";

type WindowInfo = NSDictionary;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    // CFArray/CFDictionary are toll-free bridged to NSArray/NSDictionary. The Copy result is owned.
    fn CGWindowListCopyWindowInfo(options: u32, relative_to: u32) -> *mut NSArray<WindowInfo>;
}

thread_local! {
    static OVERLAY: RefCell<Option<Overlay>> = const { RefCell::new(None) };
}

define_class!(
    #[unsafe(super(NSPanel))]
    #[thread_kind = MainThreadOnly]
    struct TopLeftPanel;

    impl TopLeftPanel {
        #[unsafe(method(canBecomeKeyWindow))]
        fn can_become_key_window(&self) -> bool { false }

        #[unsafe(method(canBecomeMainWindow))]
        fn can_become_main_window(&self) -> bool { false }

        #[unsafe(method(constrainFrameRect:toScreen:))]
        fn constrain_frame(&self, frame: NSRect, _screen: Option<&NSScreen>) -> NSRect { frame }
    }
);

struct ButtonState {
    app: AppHandle,
    config: Draft<IVerge>,
    scale: Cell<f64>,
    busy: Cell<bool>,
    title: RefCell<String>,
    reset_at: Cell<Option<Instant>>,
}

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[ivars = ButtonState]
    struct DisconnectButton;

    impl DisconnectButton {
        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool { true }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, _event: &NSEvent) {
            if !self.ivars().config.data_arc().enable_hearthstone_disconnect.unwrap_or(true)
                || self.ivars().busy.replace(true) { return; }
            self.ivars().reset_at.set(None);
            self.set_title("处理中…");
            let app = self.ivars().app.clone();
            AsyncHandler::spawn(move || async move {
                let result = super::disconnect().await.map_err(|error| error.to_string());
                let _ = app.run_on_main_thread(move || {
                    OVERLAY.with_borrow(|overlay| {
                        if let Some(overlay) = overlay {
                            overlay.button.finish(result);
                        }
                    });
                });
            });
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: NSRect) {
            let scale = self.ivars().scale.get();
            let bounds = self.bounds();
            unsafe {
                NSColor::colorWithCalibratedRed_green_blue_alpha(0.10, 0.12, 0.12, 0.95).setFill();
                NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(bounds, 5.0 * scale, 5.0 * scale).fill();
                let font = NSFont::systemFontOfSize_weight(14.0 * scale, NSFontWeightSemibold);
                let color = NSColor::colorWithCalibratedRed_green_blue_alpha(0.48, 0.86, 0.39, 1.0);
                let attrs = NSDictionary::from_slices(
                    &[NSFontAttributeName, NSForegroundColorAttributeName],
                    &[&*font as &AnyObject, &*color as &AnyObject],
                );
                let text = NSAttributedString::initWithString_attributes(
                    <NSAttributedString as objc2::AnyThread>::alloc(),
                    &NSString::from_str(&self.ivars().title.borrow()), Some(&attrs),
                );
                let size = text.size();
                text.drawAtPoint(NSPoint::new(
                    bounds.origin.x + (bounds.size.width - size.width) / 2.0,
                    bounds.origin.y + (bounds.size.height - size.height) / 2.0,
                ));
            }
        }

        #[unsafe(method(poll:))]
        fn poll(&self, _timer: &NSTimer) {
            OVERLAY.with_borrow_mut(|overlay| {
                if let Some(overlay) = overlay { overlay.poll(); }
            });
        }
    }
);

impl DisconnectButton {
    fn set_title(&self, title: &str) {
        *self.ivars().title.borrow_mut() = title.to_owned();
        self.setNeedsDisplay(true);
    }

    fn finish(&self, result: std::result::Result<(), String>) {
        self.ivars().busy.set(false);
        let (title, message) = match result {
            Ok(()) => (
                "已拔线",
                "Match connection closed; waiting for Hearthstone to reconnect.".to_owned(),
            ),
            Err(message) => ("拔线失败", message),
        };
        self.set_title(title);
        self.setToolTip(Some(&NSString::from_str(&message)));
        self.ivars().reset_at.set(Some(Instant::now() + Duration::from_secs(2)));
    }
}

struct Overlay {
    panel: Retained<TopLeftPanel>,
    button: Retained<DisconnectButton>,
    timer: Retained<NSTimer>,
    scene: GameSceneMonitor,
    last_shown: Option<bool>,
}

impl Drop for Overlay {
    fn drop(&mut self) {
        self.timer.invalidate();
        self.panel.orderOut(None);
    }
}

impl Overlay {
    fn new(app: &AppHandle, mtm: MainThreadMarker, config: Draft<IVerge>) -> Self {
        let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(104.0, 30.0));
        unsafe {
            let panel: Retained<TopLeftPanel> = msg_send![
                TopLeftPanel::alloc(mtm), initWithContentRect: frame,
                styleMask: NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel,
                backing: NSBackingStoreType::Buffered, defer: false
            ];
            panel.setReleasedWhenClosed(false);
            panel.setOpaque(false);
            panel.setBackgroundColor(Some(&NSColor::clearColor()));
            panel.setHasShadow(false);
            panel.setLevel(NSFloatingWindowLevel);
            panel.setHidesOnDeactivate(false);
            panel.setCollectionBehavior(
                NSWindowCollectionBehavior::CanJoinAllSpaces
                    | NSWindowCollectionBehavior::FullScreenAuxiliary
                    | NSWindowCollectionBehavior::IgnoresCycle,
            );
            let button = DisconnectButton::alloc(mtm).set_ivars(ButtonState {
                app: app.clone(),
                config,
                scale: Cell::new(1.0),
                busy: Cell::new(false),
                title: RefCell::new(BUTTON_TITLE.to_owned()),
                reset_at: Cell::new(None),
            });
            let button: Retained<DisconnectButton> = msg_send![super(button), initWithFrame: frame];
            button.setAutoresizingMask(
                NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
            );
            panel.setContentView(Some(&button));
            let timer =
                NSTimer::timerWithTimeInterval_target_selector_userInfo_repeats(0.3, &button, sel!(poll:), None, true);
            NSRunLoop::mainRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes);
            Self {
                panel,
                button,
                timer,
                scene: GameSceneMonitor::new(),
                last_shown: None,
            }
        }
    }

    fn poll(&mut self) {
        if self
            .button
            .ivars()
            .reset_at
            .get()
            .is_some_and(|time| Instant::now() >= time)
        {
            self.button.ivars().reset_at.set(None);
            self.button.set_title(BUTTON_TITLE);
        }
        if !self
            .button
            .ivars()
            .config
            .data_arc()
            .enable_hearthstone_disconnect
            .unwrap_or(true)
        {
            self.hide();
            return;
        }
        let Some(frame) = self.game_frame() else {
            self.hide();
            return;
        };
        let screens = NSScreen::screens(self.panel.mtm());
        let fullscreen = screens.iter().any(|screen| {
            let screen = screen.frame();
            (screen.origin.x - frame.origin.x).abs() < 2.0
                && (screen.origin.y - frame.origin.y).abs() < 2.0
                && (screen.size.width - frame.size.width).abs() < 2.0
                && (screen.size.height - frame.size.height).abs() < 2.0
        });
        let content = if fullscreen {
            frame
        } else {
            NSWindow::contentRectForFrameRect_styleMask(frame, NSWindowStyleMask::Titled, self.panel.mtm())
        };
        if !content.size.height.is_finite() || content.size.height <= 0.0 {
            self.hide();
            return;
        }
        let scale = content.size.height / 1080.0;
        if self.button.ivars().scale.replace(scale) != scale {
            self.button.setNeedsDisplay(true);
        }
        let height = 30.0 * scale;
        let target = NSRect::new(
            NSPoint::new(
                frame.origin.x + 8.0 * scale,
                frame.origin.y + frame.size.height - height,
            ),
            NSSize::new(104.0 * scale, height),
        );
        if self.panel.frame() != target {
            self.panel.setFrame_display(target, true);
        }
        if !self.panel.isVisible() {
            self.panel.orderFrontRegardless();
        }
        if self.last_shown != Some(true) {
            logging!(info, Type::Core, "Hearthstone overlay shown: {target:?}");
            self.last_shown = Some(true);
        }
    }

    fn hide(&mut self) {
        self.panel.orderOut(None);
        if self.last_shown != Some(false) {
            logging!(info, Type::Core, "Hearthstone overlay hidden");
            self.last_shown = Some(false);
        }
    }

    fn game_frame(&mut self) -> Option<NSRect> {
        let client = NSRunningApplication::runningApplicationsWithBundleIdentifier(&NSString::from_str(GAME_BUNDLE))
            .iter()
            .find(|app| !app.isTerminated())?;
        let foreground = NSWorkspace::sharedWorkspace()
            .frontmostApplication()?
            .bundleIdentifier()?
            .to_string();
        if !matches!(foreground.as_str(), GAME_BUNDLE | TRACKER_BUNDLE) || !self.scene.is_in_match() {
            return None;
        }
        let primary = NSScreen::screens(self.panel.mtm()).firstObject()?;
        // Only on-screen windows, excluding desktop elements; the retained bridge consumes Copy ownership.
        let windows = unsafe { Retained::from_raw(CGWindowListCopyWindowInfo(1 | 16, 0)) }?;
        let rect = windows
            .iter()
            .filter_map(|info| {
                if number(&info, "kCGWindowOwnerPID")?.intValue() != client.processIdentifier()
                    || number(&info, "kCGWindowLayer")?.intValue() != 0
                {
                    return None;
                }
                let bounds = info.objectForKey(&NSString::from_str("kCGWindowBounds"))?;
                let bounds = bounds.downcast_ref::<WindowInfo>()?;
                let rect = NSRect::new(
                    NSPoint::new(number(bounds, "X")?.doubleValue(), number(bounds, "Y")?.doubleValue()),
                    NSSize::new(
                        number(bounds, "Width")?.doubleValue(),
                        number(bounds, "Height")?.doubleValue(),
                    ),
                );
                (rect.size.width > 300.0 && rect.size.height > 200.0).then_some(rect)
            })
            .max_by(|a, b| (a.size.width * a.size.height).total_cmp(&(b.size.width * b.size.height)))?;
        Some(NSRect::new(
            NSPoint::new(
                rect.origin.x,
                primary.frame().origin.y + primary.frame().size.height - rect.origin.y - rect.size.height,
            ),
            rect.size,
        ))
    }
}

fn number(info: &WindowInfo, key: &str) -> Option<Retained<NSNumber>> {
    info.objectForKey(&NSString::from_str(key))?.downcast().ok()
}

pub fn start(app: &AppHandle) -> Result<()> {
    let mtm = MainThreadMarker::new().context("Hearthstone overlay must start on the main thread")?;
    let config = AsyncHandler::block_on(Config::verge());
    OVERLAY.with_borrow_mut(|overlay| {
        if overlay.is_none() {
            *overlay = Some(Overlay::new(app, mtm, config));
        }
    });
    Ok(())
}
