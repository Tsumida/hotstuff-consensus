use serde::{Deserialize, Serialize};

use crate::{ReplicaID, ViewNumber};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Context {
    pub from: ReplicaID,
    pub to: DeliveryType,

    // Latest view.
    pub view: ViewNumber,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DeliveryType {
    // To other replicas.
    Broadcast,

    // To a specified replica.
    Single(ReplicaID),

    // To someone the replica don't know.
    Unknown,
}

impl Context {
    #[inline(always)]
    pub fn size(&self) -> usize {
        self.from.len() + std::mem::size_of::<ViewNumber>()
    }

    #[inline(always)]
    pub fn new(from: ReplicaID, to: DeliveryType, view: ViewNumber) -> Context {
        Context { from, to, view }
    }

    pub fn single(from: ReplicaID, to: ReplicaID, view: ViewNumber) -> Context {
        Context {
            from,
            to: DeliveryType::Single(to),
            view,
        }
    }

    pub fn broadcast(from: ReplicaID, view: ViewNumber) -> Context {
        Context {
            from,
            to: DeliveryType::Broadcast,
            view,
        }
    }

    /// Return internal state.
    pub fn response(from: ReplicaID, view: ViewNumber) -> Context {
        Context {
            from,
            to: DeliveryType::Unknown,
            view,
        }
    }
}
