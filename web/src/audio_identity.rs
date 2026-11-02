//! Cache identities only for immutable shared audio allocations.
use sha2::{Digest, Sha256};
use std::sync::{Arc, Weak};

type Entry = (Weak<Vec<u8>>, String);
thread_local! {static HASHES:std::cell::RefCell<std::collections::BTreeMap<usize,Entry>>=std::cell::RefCell::default();}

pub fn hash(bytes: &Arc<Vec<u8>>) -> String {
    let key = Arc::as_ptr(bytes) as usize;
    if let Some(hash) = HASHES.with(|cache| {
        cache.borrow().get(&key).and_then(|(weak, hash)| {
            weak.upgrade()
                .filter(|old| Arc::ptr_eq(old, bytes))
                .map(|_| hash.clone())
        })
    }) {
        return hash;
    }
    let hash = format!("{:x}", Sha256::digest(bytes.as_slice()));
    remember_verified(bytes, &hash);
    hash
}

/// Call only after hashing these bytes or receiving verified worker output.
/// Weak ownership prevents allocator address reuse from confusing identities.
pub(crate) fn remember_verified(bytes: &Arc<Vec<u8>>, hash: &str) {
    if bytes.len() < 65536 {
        return;
    }
    HASHES.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.len() > 4096 {
            cache.retain(|_, (weak, _)| weak.strong_count() > 0);
        }
        cache.insert(
            Arc::as_ptr(bytes) as usize,
            (Arc::downgrade(bytes), hash.into()),
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_on_write_cannot_reuse_the_previous_audio_identity() {
        let mut original = Arc::new(vec![7; 65536]);
        let old_hash = hash(&original);
        let retained = original.clone();
        Arc::make_mut(&mut original)[0] = 8;
        assert_ne!(hash(&original), old_hash);
        assert_eq!(hash(&retained), old_hash);
    }

    #[test]
    fn unique_mutation_invalidates_cached_weak_identity() {
        let mut bytes = Arc::new(vec![7; 65536]);
        let old_hash = hash(&bytes);
        assert!(Arc::get_mut(&mut bytes).is_none());
        Arc::make_mut(&mut bytes)[0] = 8;
        assert_ne!(hash(&bytes), old_hash);
        assert_eq!(
            hash(&bytes),
            format!("{:x}", Sha256::digest(bytes.as_slice()))
        );
    }
}
