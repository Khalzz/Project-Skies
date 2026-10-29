//! A map keyed by type (`TypeId`) - what a node's properties, and its
//! physics inputs and states, are kept in.
//!
//! A `TypeId` is already a unique number made by the compiler, so it needs
//! no hashing of its own: `TypeIdHasher` takes it as it is, instead of the
//! standard map's general-purpose hash (built to stand up to keys chosen by
//! an attacker - nothing a type can be). The same map, cheaper to look up.

use std::any::TypeId;
use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

/// A `HashMap` by `TypeId`, made with `TypeMap::default()`.
pub type TypeMap<V> = HashMap<TypeId, V, BuildHasherDefault<TypeIdHasher>>;

/// Hands a `TypeId` on as its own hash - see the module doc.
#[derive(Default)]
pub struct TypeIdHasher(u64);

impl Hasher for TypeIdHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write_u64(&mut self, value: u64) {
        self.0 = value;
    }

    // However a TypeId writes itself - every byte goes in.
    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 = self.0.rotate_left(8) ^ u64::from(*byte);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_type_has_its_own_entry() {
        let mut map: TypeMap<&str> = TypeMap::default();
        map.insert(TypeId::of::<u32>(), "u32");
        map.insert(TypeId::of::<f32>(), "f32");
        map.insert(TypeId::of::<String>(), "String");
        assert_eq!(map[&TypeId::of::<u32>()], "u32");
        assert_eq!(map[&TypeId::of::<f32>()], "f32");
        assert_eq!(map[&TypeId::of::<String>()], "String");
        assert!(!map.contains_key(&TypeId::of::<u64>()));
    }
}
