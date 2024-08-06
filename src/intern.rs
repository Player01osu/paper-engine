use std::mem::MaybeUninit;
use std::sync::{Once, RwLock};
use std::collections::HashMap;
use std::fmt::Formatter;

static mut POOL: MaybeUninit<RwLock<StringPool>> = MaybeUninit::uninit();
static ONCE: Once = Once::new();

#[derive(Clone, Copy, Eq, PartialEq, Hash)]
pub struct PoolId (usize);

struct StringPool {
    flat_pool: Vec<&'static str>,
    map_pool: HashMap<&'static str, usize>,
}

impl std::fmt::Debug for PoolId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let id = self.0;
        let s = get_str(*self);
        write!(f, "{{id: {id}, str: {s}}}")
    }
}

impl std::fmt::Display for PoolId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let s = get_str(*self);
        write!(f, "{s}")
    }
}

pub fn intern(s: impl AsRef<str>) -> PoolId {
    ONCE.call_once(|| unsafe {
        POOL.write(RwLock::new(StringPool {
            flat_pool: vec![],
            map_pool: HashMap::new(),
        }));
    });

    let s = s.as_ref();
    let pool = unsafe { POOL.assume_init_mut() };
    if let Some(id) = pool.read().unwrap().map_pool.get(s) {
        return PoolId(*id);
    }

    let mut pool_write = pool.write().unwrap();
    let id = pool_write.flat_pool.len();
    let leaked_s = Box::leak(s.into());
    pool_write.flat_pool.push(leaked_s);
    pool_write.map_pool.insert(leaked_s, id);
    PoolId(id)
}

pub fn get_str(PoolId(id): PoolId) -> &'static str {
    assert!(ONCE.is_completed(), "Pool is not initialized; must call `intern(..)` at least once");
    let pool = unsafe { POOL.assume_init_ref() };
    let s = pool.read().unwrap().flat_pool[id];
    s
}
