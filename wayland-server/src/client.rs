use std::sync::Arc;

use wayland_backend::{
    protocol::ProtocolError,
    server::{ClientId, DisconnectReason, InvalidId},
};

use crate::{dispatch::ResourceData, Dispatch, DisplayHandle, Resource};

/// A handle connected to the server.
#[derive(Debug)]
pub struct Client {
    pub(crate) id: ClientId,
    pub(crate) data: Arc<dyn std::any::Any + Send + Sync>,
}

impl Client {
    pub(crate) fn from_id<D>(
        handle: &mut DisplayHandle<'_, D>,
        id: ClientId,
    ) -> Result<Client, InvalidId> {
        let data = handle.inner.handle().get_client_data(id.clone())?.into_any_arc();
        Ok(Client { id, data })
    }

    /// Returns the id of the client.
    pub fn id(&self) -> ClientId {
        self.id.clone()
    }

    /// Returns the data associated with the client.
    pub fn get_data<Data: 'static>(&self) -> Option<&Data> {
        (&*self.data).downcast_ref()
    }

    /// Creates a new resource for this client
    ///
    /// ## Warning about compositor created objects
    ///
    /// To ensure the state coherence between the client and server, this resource should immediately be sent
    /// to the client through an appropriate event. Failing to do so will likely result in protocol errors.
    pub fn create_resource<I: Resource + 'static, D: Dispatch<I> + 'static>(
        &self,
        handle: &mut DisplayHandle<'_, D>,
        version: u32,
        user_data: <D as Dispatch<I>>::UserData,
    ) -> Result<I, InvalidId> {
        let id = handle.inner.handle().create_object(
            self.id.clone(),
            I::interface(),
            version,
            Arc::new(ResourceData::<I, _>::new(user_data)),
        )?;
        I::from_id(handle, id)
    }

    /// Posts an error to the client's display.
    ///
    /// This function will cause a protocol error  on the client's display and the client will be
    /// disconnected.
    ///
    /// This should only be used for display level protocol errors such as a malformed request, the server
    /// running out of memory or compositor errors. Generally you will want to post an error on the resource
    /// that has caused the error using [`Resource::post_error`].
    pub fn post_error<D>(&self, handle: &mut DisplayHandle<'_, D>, error: ProtocolError) {
        handle.inner.handle().kill_client(self.id.clone(), DisconnectReason::ProtocolError(error))
    }

    /// Disconnects this client from the server.
    pub fn disconnect<D>(&self, handle: &mut DisplayHandle<'_, D>) {
        handle.inner.handle().kill_client(self.id.clone(), DisconnectReason::ConnectionClosed)
    }
}
