mod helpers;

use helpers::{roundtrip, wayc, ways, DumbClientData, TestClient, TestServer};

use ways::protocol::wl_output::{Request as ServerOutputRequest, WlOutput as ServerOutput};

use std::os::unix::io::IntoRawFd;
use std::sync::Arc;

fn main() {
    let mut server = TestServer::new();
    server.display.handle().create_global::<ServerOutput>(2, ());

    let (s1, s2) = ::std::os::unix::net::UnixStream::pair().unwrap();

    let my_client = server.display.insert_client(s1, Arc::new(DumbClientData)).unwrap();

    let fd2 = s2.into_raw_fd();
    ::std::env::set_var("WAYLAND_SOCKET", format!("{}", fd2));

    let mut client = TestClient::new_from_env();

    let mut globals = wayc::globals::GlobalList::new();

    client.display.get_registry(&mut client.cx.handle(), &client.event_queue.handle()).unwrap();

    roundtrip(&mut client, &mut server, &mut globals, &mut ServerData).unwrap();
    // check that we connected to the right compositor
    assert!(globals.list().len() == 1);
    let output = &globals.list()[0];
    assert_eq!(output.name, 1);
    assert_eq!(output.interface, "wl_output");
    assert_eq!(output.version, 2);

    my_client.kill(
        &mut server.display.handle(),
        ways::backend::protocol::ProtocolError {
            code: 0,
            object_id: 1,
            object_interface: "wl_display".into(),
            message: "I don't like you!".into(),
        },
    );

    assert!(roundtrip(&mut client, &mut server, &mut globals, &mut ServerData).is_err());
}

struct ServerData;

impl ways::Dispatch<ServerOutput> for ServerData {
    type UserData = ();
    fn request(
        &mut self,
        _: &ways::Client,
        _: &ServerOutput,
        _: ServerOutputRequest,
        _: &(),
        _: &mut ways::DisplayHandle<'_, Self>,
    ) {
    }
}

impl ways::GlobalDispatch<ServerOutput> for ServerData {
    type GlobalData = ();

    fn bind(
        &mut self,
        _: &mut ways::DisplayHandle<'_, Self>,
        _: &ways::Client,
        _: &ServerOutput,
        _: &(),
        _: &(),
    ) {
    }
}