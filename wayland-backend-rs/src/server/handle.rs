use std::{ffi::CString, sync::Arc};

use smallvec::SmallVec;
use wayland_commons::{
    core_interfaces::{WL_DISPLAY_INTERFACE, WL_REGISTRY_INTERFACE},
    same_interface,
    server::{
        BackendHandle, ClientData, DisconnectReason, GlobalHandler, GlobalInfo, InvalidId,
        ObjectData, ServerBackend,
    },
    Argument, Interface, ObjectInfo, ANONYMOUS_INTERFACE,
};

use super::{client::ClientStore, registry::Registry, ClientId, Data, GlobalId, ObjectId};
use crate::map::Object;

pub struct Handle<D, B> {
    pub(crate) clients: ClientStore<D, B>,
    pub(crate) registry: Registry<D, B>,
}

enum DispatchAction<D, B: ServerBackend<D>> {
    Request {
        object: Object<Data<D, B>>,
        object_id: B::ObjectId,
        opcode: u16,
        arguments: SmallVec<[Argument<B::ObjectId>; 4]>,
        is_destructor: bool,
    },
    Bind {
        object: B::ObjectId,
        client: B::ClientId,
        global: B::GlobalId,
        handler: Arc<dyn GlobalHandler<D, B>>,
    },
}

impl<D, B> Handle<D, B>
where
    B: ServerBackend<
        D,
        ClientId = ClientId,
        ObjectId = ObjectId,
        GlobalId = GlobalId,
        Handle = Self,
    >,
{
    pub(crate) fn new() -> Self {
        let debug = match std::env::var_os("WAYLAND_DEBUG") {
            Some(str) if str == "1" || str == "server" => true,
            _ => false,
        };
        Handle { clients: ClientStore::new(debug), registry: Registry::new() }
    }

    pub(crate) fn cleanup(&mut self) {
        let dead_clients = self.clients.cleanup();
        self.registry.cleanup(&dead_clients);
    }

    pub(crate) fn dispatch_events_for(
        &mut self,
        data: &mut D,
        client_id: ClientId,
    ) -> std::io::Result<usize> {
        let mut dispatched = 0;
        loop {
            let action = if let Ok(client) = self.clients.get_client_mut(client_id) {
                let (message, object) = match client.next_request() {
                    Ok(v) => v,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        if dispatched > 0 {
                            break;
                        } else {
                            return Err(e);
                        }
                    }
                    Err(e) => return Err(e),
                };
                dispatched += 1;
                if same_interface(object.interface, &WL_DISPLAY_INTERFACE) {
                    client.handle_display_request(message, &mut self.registry, data);
                    continue;
                } else if same_interface(object.interface, &WL_REGISTRY_INTERFACE) {
                    if let Some((client, global, object, handler)) =
                        client.handle_registry_request(message, &mut self.registry, data)
                    {
                        DispatchAction::Bind { client, global, object, handler }
                    } else {
                        continue;
                    }
                } else {
                    let object_id = ObjectId {
                        id: message.sender_id,
                        serial: object.data.serial,
                        interface: object.interface,
                        client_id: client.id,
                    };
                    let opcode = message.opcode;
                    let (arguments, is_destructor) =
                        match client.process_request(&object, message, data) {
                            Some(args) => args,
                            None => continue,
                        };
                    // Return the whole set to invoke the callback while handle is not borrower via client
                    DispatchAction::Request { object, object_id, opcode, arguments, is_destructor }
                }
            } else {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "Invalid client ID",
                ));
            };
            match action {
                DispatchAction::Request { object, object_id, opcode, arguments, is_destructor } => {
                    object
                        .data
                        .user_data
                        .request(self, data, client_id, object_id, opcode, &arguments);
                    if is_destructor {
                        object.data.user_data.destroyed(client_id, object_id);
                        if let Ok(client) = self.clients.get_client_mut(client_id) {
                            client.send_delete_id(object_id);
                        }
                    }
                }
                DispatchAction::Bind { object, client, global, handler } => {
                    handler.bind(self, data, client, global, object);
                }
            }
        }
        Ok(dispatched)
    }

    pub(crate) fn flush(&mut self, client: Option<ClientId>) -> std::io::Result<()> {
        if let Some(client) = client {
            match self.clients.get_client_mut(client) {
                Ok(client) => client.flush(),
                Err(InvalidId) => Ok(()),
            }
        } else {
            for client in self.clients.clients_mut() {
                let _ = client.flush();
            }
            Ok(())
        }
    }
}

impl<D, B> BackendHandle<D, B> for Handle<D, B>
where
    B: ServerBackend<D, ClientId = ClientId, ObjectId = ObjectId, GlobalId = GlobalId>,
{
    fn object_info(&self, id: B::ObjectId) -> Result<ObjectInfo, InvalidId> {
        self.clients.get_client(id.client_id)?.object_info(id)
    }

    fn get_client(&self, id: B::ObjectId) -> Result<ClientId, InvalidId> {
        if self.clients.get_client(id.client_id).is_ok() {
            Ok(id.client_id)
        } else {
            Err(InvalidId)
        }
    }

    fn get_client_data(&self, id: B::ClientId) -> Result<Arc<dyn ClientData<D, B>>, InvalidId> {
        let client = self.clients.get_client(id)?;
        Ok(client.data.clone())
    }

    fn all_clients<'a>(&'a self) -> Box<dyn Iterator<Item = B::ClientId> + 'a> {
        Box::new(self.clients.all_clients_id())
    }

    fn all_objects_for<'a>(
        &'a self,
        client_id: B::ClientId,
    ) -> Result<Box<dyn Iterator<Item = B::ObjectId> + 'a>, InvalidId> {
        let client = self.clients.get_client(client_id)?;
        Ok(Box::new(client.all_objects()))
    }

    fn create_object(
        &mut self,
        client_id: ClientId,
        interface: &'static Interface,
        version: u32,
        data: Arc<dyn ObjectData<D, B>>,
    ) -> Result<ObjectId, InvalidId> {
        let client = self.clients.get_client_mut(client_id)?;
        Ok(client.create_object(interface, version, data))
    }

    fn null_id(&mut self) -> ObjectId {
        ObjectId {
            id: 0,
            serial: 0,
            client_id: ClientId { id: 0, serial: 0 },
            interface: &ANONYMOUS_INTERFACE,
        }
    }

    fn send_event(
        &mut self,
        object_id: ObjectId,
        opcode: u16,
        args: &[Argument<ObjectId>],
    ) -> Result<(), InvalidId> {
        self.clients.get_client_mut(object_id.client_id)?.send_event(object_id, opcode, args)
    }

    fn get_object_data(&self, id: ObjectId) -> Result<Arc<dyn ObjectData<D, B>>, InvalidId> {
        self.clients.get_client(id.client_id)?.get_object_data(id)
    }

    fn post_error(&mut self, object_id: ObjectId, error_code: u32, message: CString) {
        if let Ok(client) = self.clients.get_client_mut(object_id.client_id) {
            client.post_error(object_id, error_code, message)
        }
    }

    fn kill_client(&mut self, client_id: ClientId, reason: DisconnectReason) {
        if let Ok(client) = self.clients.get_client_mut(client_id) {
            client.kill(reason)
        }
    }

    fn create_global(
        &mut self,
        interface: &'static Interface,
        version: u32,
        handler: Arc<dyn GlobalHandler<D, B>>,
    ) -> GlobalId {
        self.registry.create_global(interface, version, handler, &mut self.clients)
    }

    fn disable_global(&mut self, id: GlobalId) {
        self.registry.disable_global(id, &mut self.clients)
    }

    fn remove_global(&mut self, id: GlobalId) {
        self.registry.remove_global(id, &mut self.clients)
    }

    fn global_info(&self, id: B::GlobalId) -> Result<GlobalInfo, InvalidId> {
        self.registry.get_info(id)
    }

    fn get_global_handler(
        &self,
        id: B::GlobalId,
    ) -> Result<Arc<dyn GlobalHandler<D, B>>, InvalidId> {
        self.registry.get_handler(id)
    }
}
