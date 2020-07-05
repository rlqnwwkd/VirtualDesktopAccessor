// Some reason the co_class macro uses null comparison
#![allow(clippy::cmp_null)]

use com::{co_class, interfaces::IUnknown, ComRc};

use crate::{
    get_desktops, get_index_by_desktop,
    hresult::HRESULT,
    interfaces::{
        IApplicationView, IVirtualDesktop, IVirtualDesktopNotification,
        IVirtualDesktopNotificationService,
    },
    DesktopID, HWND,
};
use crossbeam_channel::{Receiver, Sender};

pub enum VirtualDesktopEvent {
    DesktopCreated(usize),
    DesktopDestroyed(usize),
    DesktopChanged(usize, usize),
    WindowChanged(HWND),
}

pub struct RegisteredListener {
    // This is the value for registrations and unregistrations
    cookie: u32,

    // Listener holds the value on which the IVirtualDesktopNotificationService points
    #[allow(dead_code)]
    listener: Box<VirtualDesktopChangeListener>,

    // Receiver
    receiver: Receiver<VirtualDesktopEvent>,

    // Unregistration on drop requires a notification service
    service: ComRc<dyn IVirtualDesktopNotificationService>,
}
unsafe impl Send for RegisteredListener {}
unsafe impl Sync for RegisteredListener {}

impl RegisteredListener {
    pub fn register(
        sender: Sender<VirtualDesktopEvent>,
        receiver: Receiver<VirtualDesktopEvent>,
        service: ComRc<dyn IVirtualDesktopNotificationService>,
    ) -> Result<RegisteredListener, HRESULT> {
        let listener = VirtualDesktopChangeListener::create(sender);
        let ptr: ComRc<dyn IVirtualDesktopNotification> = unsafe {
            ComRc::from_raw(&listener.__ivirtualdesktopnotificationvptr as *const _ as *mut _)
        };

        // Register the IVirtualDesktopNotification to the service
        let mut cookie = 0;
        let res = unsafe { service.register(ptr.clone(), &mut cookie) };
        if res.failed() {
            #[cfg(feature = "debug")]
            println!("Registration failed {:?}", res);

            Err(res)
        } else {
            #[cfg(feature = "debug")]
            println!(
                "Register a listener {:?} {:?} {:?}",
                listener.__refcnt,
                cookie,
                std::thread::current().id()
            );

            Ok(RegisteredListener {
                cookie,
                listener,
                receiver,
                service: service.clone(),
            })
        }
    }

    pub fn get_receiver(&self) -> Receiver<VirtualDesktopEvent> {
        self.receiver.clone()
    }
}

impl Drop for RegisteredListener {
    fn drop(&mut self) {
        #[cfg(feature = "debug")]
        println!("Unregister a listener {:?}", self.cookie);
        unsafe {
            self.service.unregister(self.cookie);
        }
    }
}

#[co_class(implements(IVirtualDesktopNotification))]
struct VirtualDesktopChangeListener {
    sender: Sender<VirtualDesktopEvent>,
}

impl VirtualDesktopChangeListener {
    // Notice that com-rs package requires empty new, even though it's not used
    // for anything in this case, because we are not creating a COM server
    fn new() -> Box<VirtualDesktopChangeListener> {
        panic!()
        // VirtualDesktopChangeListener::allocate()
    }

    fn create(sender: Sender<VirtualDesktopEvent>) -> Box<VirtualDesktopChangeListener> {
        let v = VirtualDesktopChangeListener::allocate(sender);
        unsafe {
            v.add_ref();
        }
        v
    }
}

impl Drop for VirtualDesktopChangeListener {
    fn drop(&mut self) {
        #[cfg(feature = "debug")]
        println!("Drop VirtualDesktopChangeListener");
        unsafe {
            self.release();
        }
    }
}

impl IVirtualDesktopNotification for VirtualDesktopChangeListener {
    /// On desktop creation
    unsafe fn virtual_desktop_created(&self, desktop: ComRc<dyn IVirtualDesktop>) -> HRESULT {
        let mut id: DesktopID = Default::default();
        desktop.get_id(&mut id);
        if let Ok(index) = get_index_by_desktop(id) {
            let _ = self
                .sender
                .try_send(VirtualDesktopEvent::DesktopCreated(index));
        }
        HRESULT::ok()
    }

    /// On desktop destroy begin
    unsafe fn virtual_desktop_destroy_begin(
        &self,
        _destroyed_desktop: ComRc<dyn IVirtualDesktop>,
        _fallback_desktop: ComRc<dyn IVirtualDesktop>,
    ) -> HRESULT {
        HRESULT::ok()
    }

    /// On desktop destroy failed
    unsafe fn virtual_desktop_destroy_failed(
        &self,
        _destroyed_desktop: ComRc<dyn IVirtualDesktop>,
        _fallback_desktop: ComRc<dyn IVirtualDesktop>,
    ) -> HRESULT {
        HRESULT::ok()
    }

    /// On desktop destory
    unsafe fn virtual_desktop_destroyed(
        &self,
        destroyed_desktop: ComRc<dyn IVirtualDesktop>,
        _fallback_desktop: ComRc<dyn IVirtualDesktop>,
    ) -> HRESULT {
        let mut id: DesktopID = Default::default();
        destroyed_desktop.get_id(&mut id);

        // TODO: Can this work, should I move this to destroy begin?
        if let Ok(index) = get_index_by_desktop(id) {
            let _ = self
                .sender
                .try_send(VirtualDesktopEvent::DesktopDestroyed(index));
        }
        HRESULT::ok()
    }

    /// On view/window change
    unsafe fn view_virtual_desktop_changed(&self, view: ComRc<dyn IApplicationView>) -> HRESULT {
        let mut hwnd = 0 as _;
        view.get_thumbnail_window(&mut hwnd);

        #[cfg(feature = "debug")]
        println!(
            "-> Window changed {:?} {:?}",
            hwnd,
            std::thread::current().id()
        );

        let _ = self
            .sender
            .try_send(VirtualDesktopEvent::WindowChanged(hwnd));

        HRESULT::ok()
    }

    /// On desktop change
    unsafe fn current_virtual_desktop_changed(
        &self,
        old_desktop: ComRc<dyn IVirtualDesktop>,
        new_desktop: ComRc<dyn IVirtualDesktop>,
    ) -> HRESULT {
        let mut old_id: DesktopID = Default::default();
        let mut new_id: DesktopID = Default::default();
        old_desktop.get_id(&mut old_id);
        new_desktop.get_id(&mut new_id);

        #[cfg(feature = "debug")]
        println!("-> Desktop changed {:?}", std::thread::current().id());

        // Get desktop indices and notify back
        if let Ok(desktops) = get_desktops() {
            let mut old = std::usize::MAX;
            let mut new = std::usize::MAX;
            for (i, desktop) in desktops.iter().enumerate() {
                if desktop == &old_id {
                    old = i;
                } else if desktop == &new_id {
                    new = i;
                }
            }
            if old != std::usize::MAX && new != std::usize::MAX {
                let _ = self
                    .sender
                    .try_send(VirtualDesktopEvent::DesktopChanged(old, new));
            }
        }
        HRESULT::ok()
    }
}
