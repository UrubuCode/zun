use std::hint::black_box;
use std::time::Instant;

use zun_core::{Any, OrderedMap};

fn main() {
    for &count in &[200_000usize, 1_000_000] {
        let mut map = OrderedMap::new();
        let start = Instant::now();
        for key in 0..count {
            let key = key as i64;
            map.insert(Any::int(key), Any::int(key), key as u64, |candidate| {
                candidate.as_int() == Some(key)
            });
        }
        let insert_ns = start.elapsed().as_nanos() as f64 / count as f64;

        let start = Instant::now();
        let mut found = 0usize;
        for key in 0..count {
            let key = key as i64;
            if map
                .get(key as u64, |candidate| candidate.as_int() == Some(key))
                .is_some()
            {
                found += 1;
            }
        }
        black_box(found);
        let probe_ns = start.elapsed().as_nanos() as f64 / count as f64;
        println!("N={count}: insert={insert_ns:.3} ns/op, probe={probe_ns:.3} ns/op");
    }
}
