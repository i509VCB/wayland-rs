use std::{
    os::unix::net::UnixStream,
    sync::{Arc, Mutex, MutexGuard},
};

use wayland_backend::{
    protocol::ObjectInfo,
    server::{Backend, ClientData, GlobalId, Handle, InitError, InvalidId, ObjectData, ObjectId},
};

use crate::{
    global::{GlobalData, GlobalDispatch},
    Client, Resource,
};

/// The wayland display.
///
/// Since this object is the core of a server, it must be kept alive as long as the server is running.
///
/// A display represents the server side state machine in the Wayland protocol. The display will process
/// requests and send events to a client's objects.
#[derive(Debug, Clone)]
pub struct Display<D> {
    backend: Arc<Mutex<Backend<D>>>,
}

impl<D> Display<D> {
    /// Creates a new display.
    ///
    /// Note that creating a display does not mean the server is ready to receive connections. You will need
    /// to add listening sockets for the display.
    pub fn new() -> Result<Display<D>, InitError> {
        Ok(Display { backend: Arc::new(Mutex::new(Backend::new()?)) })
    }

    /// Returns a handle to the display.
    ///
    /// The handle is passed around to send events to clients and manage globals advertised by the server.
    pub fn handle(&self) -> DisplayHandle<'_, D> {
        DisplayHandle { inner: HandleInner::Guard(self.backend.lock().unwrap()) }
    }

    /// Creates a client from a unix socket.
    pub fn insert_client(
        &mut self,
        stream: UnixStream,
        data: Arc<dyn ClientData<D>>,
    ) -> std::io::Result<Client> {
        let id = self.backend.lock().unwrap().insert_client(stream, data.clone())?;
        Ok(Client { id, data: data.into_any_arc() })
    }

    /// Dispatches all pending requests from clients.
    ///
    /// This function will not block if there are no pending messages.
    ///
    /// The provided data will be provided to the handler of messages received from the clients.
    ///
    /// For performance reasons, use of this function should be integrated with an event loop, monitoring the
    /// poll file descriptor and only calling this method when messages are available.
    pub fn dispatch_clients(&mut self, data: &mut D) -> std::io::Result<usize> {
        self.backend.lock().unwrap().dispatch_all_clients(data)
    }

    /// Flushes pending events destined for all clients.
    pub fn flush_clients(&mut self) -> std::io::Result<()> {
        self.backend.lock().unwrap().flush(None)
    }
}

/// A handle to a wayland display.
#[derive(Debug)]
pub struct DisplayHandle<'a, D> {
    pub(crate) inner: HandleInner<'a, D>,
}

#[derive(Debug)]
pub(crate) enum HandleInner<'a, D> {
    Handle(&'a mut Handle<D>),
    Guard(MutexGuard<'a, Backend<D>>),
}

impl<'a, D> HandleInner<'a, D> {
    #[inline]
    pub(crate) fn handle(&mut self) -> &mut Handle<D> {
        match self {
            HandleInner::Handle(handle) => handle,
            HandleInner::Guard(guard) => guard.handle(),
        }
    }
}

impl<'a, D> DisplayHandle<'a, D> {
    pub(crate) fn from_handle(handle: &'a mut Handle<D>) -> DisplayHandle<'a, D> {
        DisplayHandle { inner: HandleInner::Handle(handle) }
    }

    /// Returns the user data associated with some object.
    pub fn get_object_data(&mut self, id: ObjectId) -> Result<Arc<dyn ObjectData<D>>, InvalidId> {
        self.inner.handle().get_object_data(id)
    }

    /// Returns information about some object.
    pub fn object_info(&mut self, id: ObjectId) -> Result<ObjectInfo, InvalidId> {
        self.inner.handle().object_info(id)
    }

    /// Returns an object id that represents a null object.
    pub fn null_id(&mut self) -> ObjectId {
        self.inner.handle().null_id()
    }

    /// Sends an event to the object.
    ///
    /// The result will be [`Err`] if the associated client has been disconnected.
    pub fn send_event<I: Resource>(
        &mut self,
        resource: &I,
        event: I::Event,
    ) -> Result<(), InvalidId> {
        let msg = resource.write_event(self, event)?;
        self.inner.handle().send_event(msg)
    }

    /// Posts a protocol error to the resource.
    ///
    /// The error can be obtained from the various `Error` enums of the protocols.
    ///
    /// Protocol errors are fatal and the client will be disconnected.
    pub fn post_error<I: Resource>(
        &mut self,
        resource: &I,
        code: impl Into<u32>,
        error: impl Into<String>,
    ) {
        self.inner.handle().post_error(
            resource.id(),
            code.into(),
            std::ffi::CString::new(error.into()).unwrap(),
        )
    }

    /// Creates a new global object.
    ///
    /// The global will be advertised to clients where [`GlobalDispatch`] returns `true`.
    ///
    /// The version parameter specified is the **highest supported version**, you must be able to handle
    /// clients that choose to instantiate this global with a lower version number.
    ///
    /// The returned [`GlobalId`] may be used to [`disable`](DisplayHandle::disable_global) and
    /// [`remove`](DisplayHandle::remove_global) the global later.
    pub fn create_global<I: Resource + 'static>(
        &mut self,
        version: u32,
        data: <D as GlobalDispatch<I>>::GlobalData,
    ) -> GlobalId
    where
        D: GlobalDispatch<I> + 'static,
    {
        self.inner.handle().create_global(I::interface(), version, Arc::new(GlobalData { data }))
    }

    /// Disables an active global.
    ///
    /// The global removal will be signaled to all currently connected clients. New clients will not know of
    /// the global.
    pub fn disable_global(&mut self, id: GlobalId) {
        self.inner.handle().disable_global(id)
    }

    pub fn remove_global(&mut self, id: GlobalId) {
        self.inner.handle().remove_global(id)
    }
}
