// This module contains helpers functions and types that
// are not test in themselves, but are used by several tests.

#![allow(dead_code, unused_macros)]

pub extern crate wayland_client as wayc;
pub extern crate wayland_server as ways;

use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use wayland_backend::client::ObjectData;

pub struct TestServer<D> {
    pub display: self::ways::Display<D>,
}

impl<D> TestServer<D> {
    pub fn new() -> TestServer<D> {
        let display = self::ways::Display::new().unwrap();
        TestServer { display }
    }

    pub fn answer(&mut self, ddata: &mut D) {
        self.display.dispatch_clients(ddata).unwrap();
        self.display.flush_clients().unwrap();
    }

    pub fn add_client_with_data<CD>(
        &mut self,
        data: Arc<dyn ways::backend::ClientData<D>>,
    ) -> (ways::Client, TestClient<CD>) {
        let (server_socket, client_socket) = UnixStream::pair().unwrap();
        let client = self.display.insert_client(server_socket, data).unwrap();
        let test_client = TestClient::new(client_socket);
        (client, test_client)
    }

    pub fn add_client<CD>(&mut self) -> (ways::Client, TestClient<CD>) {
        self.add_client_with_data(Arc::new(DumbClientData))
    }
}

pub struct TestClient<D> {
    pub cx: self::wayc::Connection,
    pub display: self::wayc::protocol::wl_display::WlDisplay,
    pub event_queue: self::wayc::EventQueue<D>,
}

impl<D> TestClient<D> {
    pub fn new(socket: UnixStream) -> TestClient<D> {
        let cx = self::wayc::Connection::from_socket(socket).expect("Failed to connect to server.");
        let event_queue = cx.new_event_queue();
        let display = cx.handle().display();
        TestClient { cx, display, event_queue }
    }

    pub fn new_from_env() -> TestClient<D> {
        let cx = self::wayc::Connection::connect_to_env().expect("Failed to connect to server.");
        let event_queue = cx.new_event_queue();
        let display = cx.handle().display();
        TestClient { cx, display, event_queue }
    }
}

pub fn roundtrip<CD: 'static, SD: 'static>(
    client: &mut TestClient<CD>,
    server: &mut TestServer<SD>,
    client_ddata: &mut CD,
    server_ddata: &mut SD,
) -> Result<(), wayc::backend::WaylandError> {
    // send to the server
    let done = Arc::new(AtomicBool::new(false));
    let done2 = done.clone();
    client
        .cx
        .handle()
        .send_request(
            &client.display,
            wayc::protocol::wl_display::Request::Sync {},
            Some(Arc::new(SyncData { done })),
        )
        .unwrap();
    while !done2.load(Ordering::Acquire) {
        match client.cx.flush() {
            Ok(_) => {}
            Err(wayc::backend::WaylandError::Io(e))
                if e.kind() == ::std::io::ErrorKind::BrokenPipe => {}
            Err(e) => return Err(e),
        }
        ::std::thread::sleep(::std::time::Duration::from_millis(100));
        // make it answer messages
        server.answer(server_ddata);
        ::std::thread::sleep(::std::time::Duration::from_millis(100));
        // dispatch all client-side
        client.event_queue.dispatch_pending(client_ddata).unwrap();
        let e = client.cx.dispatch_events();
        // even if read_events returns an error, some messages may need dispatching
        client.event_queue.dispatch_pending(client_ddata).unwrap();
        e?;
    }
    Ok(())
}

struct SyncData {
    done: Arc<AtomicBool>,
}

impl wayc::backend::ObjectData for SyncData {
    fn event(
        self: Arc<Self>,
        _handle: &mut wayc::backend::Handle,
        _msg: self::wayc::backend::protocol::Message<wayc::backend::ObjectId>,
    ) -> Option<Arc<dyn ObjectData>> {
        self.done.store(true, Ordering::Release);
        None
    }

    fn destroyed(&self, _: wayc::backend::ObjectId) {}
}

pub struct DumbClientData;

impl<D> ways::backend::ClientData<D> for DumbClientData {
    fn initialized(&self, _: ways::backend::ClientId) {}
    fn disconnected(&self, _: ways::backend::ClientId, _: ways::backend::DisconnectReason) {}
}

macro_rules! client_ignore_impl {
    ($handler:ty => [$($iface:ty),*]) => {
        $(
            impl $crate::helpers::wayc::Dispatch<$iface> for $handler {
                type UserData = ();
                fn event(
                    &mut self,
                    _: &$iface,
                    _: <$iface as $crate::helpers::wayc::Proxy>::Event,
                    _: &Self::UserData,
                    _: &mut $crate::helpers::wayc::ConnectionHandle,
                    _: &$crate::helpers::wayc::QueueHandle<Self>,
                    _: &mut $crate::helpers::wayc::DataInit<'_>,
                ) {
                }
            }
        )*
    }
}

macro_rules! server_ignore_impl {
    ($handler:ty => [$($iface:ty),*]) => {
        $(
            impl $crate::helpers::ways::Dispatch<$iface> for $handler {
                type UserData = ();
                fn request(
                    &mut self,
                    _: &$crate::helpers::ways::Client,
                    _: &$iface,
                    _: <$iface as $crate::helpers::ways::Resource>::Request,
                    _: &Self::UserData,
                    _: &mut $crate::helpers::ways::DisplayHandle<'_, Self>,
                    _: &mut $crate::helpers::ways::DataInit<'_, Self>,
                ) {
                }
            }
        )*
    }
}

macro_rules! server_ignore_global_impl {
    ($handler:ty => [$($iface:ty),*]) => {
        $(
            impl $crate::helpers::ways::GlobalDispatch<$iface> for $handler {
                type GlobalData = ();

                fn bind(
                    &mut self,
                    _: &mut $crate::helpers::ways::DisplayHandle<'_, Self>,
                    _: &$crate::helpers::ways::Client,
                    _: &$iface,
                    _: &(),
                ) -> () {
                }
            }
        )*
    }
}
