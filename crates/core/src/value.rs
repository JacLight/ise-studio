//! Type-erased component values held by the journal. Commits clone typed
//! values instead of serializing them; JSON is produced only for the wire.

use crate::registry::Persist;
use serde_json::Value;
use std::{any::Any, fmt::Debug, sync::Arc};

pub trait ErasedValue: Any + Send + Sync + Debug {
    fn as_any(&self) -> &dyn Any;
    fn to_json(&self) -> Value;
    fn eq_dyn(&self, other: &dyn ErasedValue) -> bool;
}

impl<T: Persist> ErasedValue for T {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn to_json(&self) -> Value {
        serde_json::to_value(self).expect("persistent components serialize to JSON")
    }

    fn eq_dyn(&self, other: &dyn ErasedValue) -> bool {
        other.as_any().downcast_ref::<T>().is_some_and(|o| o == self)
    }
}

pub type CompValue = Arc<dyn ErasedValue>;

pub(crate) fn same(a: Option<&CompValue>, b: Option<&CompValue>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => a.eq_dyn(b.as_ref()),
        _ => false,
    }
}
