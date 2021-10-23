use std::sync::{Arc, Mutex};

use futures_channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender};
use wayland_backend::{
    client::{Backend, Handle, ObjectData, ObjectId},
    protocol::{Message, ObjectInfo},
};

use crate::{ConnectionHandle, DispatchError, Proxy};

pub trait Dispatch<I: Proxy>: Sized {
    type UserData: Default + Send + Sync + 'static;

    fn event(
        &mut self,
        proxy: &I,
        event: I::Event,
        data: &Self::UserData,
        cxhandle: &mut ConnectionHandle,
        qhandle: &QueueHandle<Self>,
    );

    fn destroyed(&mut self, _proxy: &I, _data: &Self::UserData) {}

    fn child_from_event(_: &ObjectInfo, _: &QueueHandle<Self>) -> Arc<dyn ObjectData> {
        panic!(
            "Attempting to create an object in event from uninitialized Dispatch<{}>",
            std::any::type_name::<I>()
        );
    }
}

#[macro_export]
macro_rules! generate_child_from_event {
    ($($child_iface:ty),*) => {
        fn child_from_event(info: &$crate::backend::protocol::ObjectInfo, handle: &$crate::QueueHandle<Self>) -> std::sync::Arc<dyn $crate::backend::ObjectData> {
            match () {
                $(
                    () if $crate::backend::protocol::same_interface(info.interface, <$child_iface as $crate::Proxy>::interface()) => {
                        handle.make_data::<$child_iface>()
                    },
                )*
                _ => panic!("Attempting to create an unexpected object {:?} in event from Dispatch<{}>", info, std::any::type_name::<Self>()),
            }
        }
    }
}

type QueueCallback<D> = fn(
    &mut ConnectionHandle<'_>,
    Message<ObjectId>,
    &mut D,
    &QueueHandle<D>,
) -> Result<(), DispatchError>;
type QueueDestructor<D> = fn(&mut ConnectionHandle<'_>, ObjectId, &mut D);

enum QueueEvent<D> {
    Msg(QueueCallback<D>, Message<ObjectId>),
    Destructor(QueueDestructor<D>, ObjectId),
}

impl<D> std::fmt::Debug for QueueEvent<D> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            QueueEvent::Msg(_, ref msg) => {
                f.debug_struct("QueueEvent::Msg").field("msg", msg).finish()
            }
            QueueEvent::Destructor(_, ref id) => {
                f.debug_struct("QueueEvent::Destructor").field("id", id).finish()
            }
        }
    }
}

pub struct EventQueue<D> {
    rx: UnboundedReceiver<QueueEvent<D>>,
    handle: QueueHandle<D>,
    backend: Arc<Mutex<Backend>>,
}

impl<D> std::fmt::Debug for EventQueue<D> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventQueue")
            .field("rx", &self.rx)
            .field("handle", &self.handle)
            .finish_non_exhaustive()
    }
}

impl<D> EventQueue<D> {
    pub(crate) fn new(backend: Arc<Mutex<Backend>>) -> Self {
        let (tx, rx) = unbounded();
        EventQueue { rx, handle: QueueHandle { tx }, backend }
    }

    pub fn handle(&self) -> QueueHandle<D> {
        self.handle.clone()
    }

    pub fn dispatch_pending(&mut self, data: &mut D) -> Result<usize, DispatchError> {
        Self::dispatching_impl(&mut self.backend.lock().unwrap(), &mut self.rx, &self.handle, data)
    }

    pub fn blocking_dispatch(&mut self, data: &mut D) -> Result<usize, DispatchError> {
        let mut backend = self.backend.lock().unwrap();
        let dispatched = Self::dispatching_impl(&mut backend, &mut self.rx, &self.handle, data)?;
        if dispatched > 0 {
            Ok(dispatched)
        } else {
            crate::cx::blocking_dispatch_impl(&mut backend)?;
            Self::dispatching_impl(&mut backend, &mut self.rx, &self.handle, data)
        }
    }

    fn dispatching_impl(
        backend: &mut Backend,
        rx: &mut UnboundedReceiver<QueueEvent<D>>,
        qhandle: &QueueHandle<D>,
        data: &mut D,
    ) -> Result<usize, DispatchError> {
        let mut handle = ConnectionHandle::from_handle(backend.handle());
        let mut dispatched = 0;

        while let Ok(Some(evt)) = rx.try_next() {
            match evt {
                QueueEvent::Msg(cb, msg) => {
                    cb(&mut handle, msg, data, qhandle)?;
                    dispatched += 1;
                }
                QueueEvent::Destructor(cb, id) => {
                    cb(&mut handle, id, data);
                }
            }
        }
        Ok(dispatched)
    }
}

pub struct QueueHandle<D> {
    tx: UnboundedSender<QueueEvent<D>>,
}

impl<Data> std::fmt::Debug for QueueHandle<Data> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QueueHandle").field("tx", &self.tx).finish()
    }
}

impl<Data> Clone for QueueHandle<Data> {
    fn clone(&self) -> Self {
        QueueHandle { tx: self.tx.clone() }
    }
}

pub(crate) struct QueueSender<D> {
    func: QueueCallback<D>,
    dest: QueueDestructor<D>,
    pub(crate) handle: QueueHandle<D>,
}

impl<D> QueueSender<D> {
    fn send(&self, msg: Message<ObjectId>) {
        if self.handle.tx.unbounded_send(QueueEvent::Msg(self.func, msg)).is_err() {
            log::error!("Event received for EventQueue after it was dropped.");
        }
    }

    fn send_destroy(&self, id: ObjectId) {
        let _ = self.handle.tx.unbounded_send(QueueEvent::Destructor(self.dest, id));
    }
}

impl<D: 'static> QueueHandle<D> {
    pub fn make_data<I: Proxy + 'static>(&self) -> Arc<dyn ObjectData>
    where
        D: Dispatch<I>,
    {
        let sender = QueueSender {
            func: queue_callback::<I, D>,
            dest: queue_destructor::<I, D>,
            handle: self.clone(),
        };
        Arc::new(QueueProxyData { sender, udata: Default::default() })
    }
}

fn queue_callback<I: Proxy, D: Dispatch<I> + 'static>(
    handle: &mut ConnectionHandle<'_>,
    msg: Message<ObjectId>,
    data: &mut D,
    qhandle: &QueueHandle<D>,
) -> Result<(), DispatchError> {
    let (proxy, event) = I::parse_event(handle, msg)?;
    let udata = proxy.data::<D>().expect("Wrong user_data value for object");
    data.event(&proxy, event, udata, handle, qhandle);
    Ok(())
}

fn queue_destructor<I: Proxy, D: Dispatch<I> + 'static>(
    handle: &mut ConnectionHandle<'_>,
    id: ObjectId,
    data: &mut D,
) {
    let proxy = I::from_id(handle, id).expect("Processing destructor of invalid id ?!");
    let udata = proxy.data::<D>().expect("Wrong user_data value for object");
    data.destroyed(&proxy, udata)
}

pub struct QueueProxyData<I: Proxy, D: Dispatch<I>> {
    pub(crate) sender: QueueSender<D>,
    pub udata: <D as Dispatch<I>>::UserData,
}

impl<I: Proxy + 'static, D: Dispatch<I> + 'static> ObjectData for QueueProxyData<I, D> {
    fn make_child(self: Arc<Self>, child_info: &ObjectInfo) -> Arc<dyn ObjectData> {
        D::child_from_event(child_info, &self.sender.handle)
    }

    fn event(&self, _: &mut Handle, msg: Message<ObjectId>) {
        self.sender.send(msg);
    }

    fn destroyed(&self, object_id: ObjectId) {
        self.sender.send_destroy(object_id);
    }
}
