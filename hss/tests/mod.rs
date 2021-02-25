//! Test

use std::time::SystemTime;

use sled::Db;

#[test]
#[ignore = "tested"]
fn flush_test() {
    let block_64 = vec![0; 64];
    let block_128 = vec![0; 128];
    let block_256 = vec![0; 256];
    let block_512 = vec![0; 512];
    let block_1k = vec![0; 1024];
    let block_10k = vec![0; 1024 * 10];
    let block_100k = vec![0; 100 * 1024];

    let db_path = "./test-output/immediate-flush-test";
    let db = sled::Config::default()
        .path(db_path)
        .temporary(true)
        .open()
        .unwrap();

    batch_test(&block_512, &db);
    batch_test(&block_256, &db);
    batch_test(&block_128, &db);
    batch_test(&block_64, &db);
    batch_test(&block_1k, &db);
    batch_test(&block_10k, &db);
    batch_test(&block_100k, &db);

    drop(db);
}

fn batch_test(block: &Vec<u8>, db: &Db) {
    let num = 5_000usize;
    let tree_1 = db.open_tree(format!("block-{}-Byte", num)).unwrap();
    let mut total = 0;
    let st = SystemTime::now();
    for i in 0..num {
        tree_1.insert(i.to_be_bytes(), block.as_slice()).unwrap();
        total += tree_1.flush().unwrap();
    }
    println!(
        "block-{} num={} takes {} ms, wrote {} KB",
        block.len(),
        num,
        SystemTime::now().duration_since(st).unwrap().as_millis(),
        total >> 10,
    );
}
