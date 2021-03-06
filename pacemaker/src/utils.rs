//! Something useful for devlopment.

use hs_data::ViewNumber;
use std::hash::Hasher;

/// Default timeout, 60 seconds.
#[inline(always)]
pub fn view_hash(view: ViewNumber, total: usize) -> usize {
    let mut h = fnv::FnvHasher::default();
    h.write_u64(view);
    h.finish() as usize % total
}

#[test]
#[ignore = "tested"]
fn hash_collision() {
    use std::time;
    use time::SystemTime;

    // test done, took 2392 ms
    // index= 0, cnt=  999871, max-step=     203
    // index= 1, cnt=  999964, max-step=     423
    // index= 2, cnt= 1000077, max-step=     124
    // index= 3, cnt= 1000014, max-step=     272
    // index= 4, cnt=  999990, max-step=     144
    // index= 5, cnt= 1000096, max-step=     183
    // index= 6, cnt=  999988, max-step=     269
    const N: usize = 3 * 2 + 1;
    let mut cnt = [0; N];
    let mut max_step = [0u64; N]; // how many steps
    let mut prev_index = [0u64; N];
    let flap_times = N as u64 * 1_000_000;

    let st = time::SystemTime::now();
    for view in 0..flap_times {
        let index = view_hash(view, N);
        let c = cnt.get_mut(index).unwrap();
        let s = max_step.get_mut(index).unwrap();
        let prev_view = prev_index.get_mut(index).unwrap();
        *s = u64::max(*s, view - *prev_view as u64);
        *c += 1;
        *prev_view = view;
    }

    let d = SystemTime::now().duration_since(st).unwrap();
    println! {"test done, took {} ms", d.as_millis()};
    for (i, (c, m)) in cnt.iter().zip(max_step.iter()).enumerate() {
        println!("index={:2}, cnt={:8}, max-step={:8}", i, c, m);
    }
}
