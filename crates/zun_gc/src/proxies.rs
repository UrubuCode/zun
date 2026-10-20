//! Managed proxy state. Targets and handlers are strong references owned by
//! the proxy record; trap dispatch is layered above this GC floor.

use super::{Heap, Obj};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProxyRecord {
    pub target: Obj,
    pub handler: Obj,
}

impl Heap {
    pub fn alloc_proxy(&mut self, target: Obj, handler: Obj) -> Obj {
        let proxy = self.alloc(0, 0);
        self.proxies.insert(proxy.addr() as usize, ProxyRecord { target, handler });
        proxy
    }

    pub fn proxy_target(&self, proxy: Obj) -> Option<Obj> {
        self.proxies.get(&(proxy.addr() as usize)).map(|record| record.target)
    }

    pub fn proxy_handler(&self, proxy: Obj) -> Option<Obj> {
        self.proxies.get(&(proxy.addr() as usize)).map(|record| record.handler)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_keeps_target_and_handler_identity() {
        let mut heap = Heap::new();
        let target = heap.alloc(1, 0);
        let handler = heap.alloc(2, 0);
        let proxy = heap.alloc_proxy(target, handler);
        assert_eq!(heap.proxy_target(proxy), Some(target));
        assert_eq!(heap.proxy_handler(proxy), Some(handler));
    }

    #[test]
    fn proxy_traces_target_and_handler_as_strong_roots() {
        let mut heap = Heap::new();
        let target = heap.alloc(1, 0);
        let handler = heap.alloc(2, 0);
        let proxy = heap.alloc_proxy(target, handler);
        assert_eq!(unsafe { heap.collect(&[proxy], &|_| Vec::new()) }, 0);
        assert_eq!(heap.proxy_target(proxy), Some(target));
        assert_eq!(heap.proxy_handler(proxy), Some(handler));
    }
}
