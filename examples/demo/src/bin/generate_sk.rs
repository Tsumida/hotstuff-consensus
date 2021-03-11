//! Generate publickey set and partial secret key.

use std::{io::Write, process::exit};

use clap::{App, Arg};
use threshold_crypto::serde_impl::SerdeSecret;

fn main() {
    // generate pkset, sks,
    let matches = App::new("threhsold-key generator")
        .version("0.1.0")
        .author("tsuko")
        .about("Generating threshold key for testing")
        .arg(
            Arg::with_name("num")
                .short("n")
                .long("num")
                .takes_value(true)
                .help("The number of hotstuff peers"),
        )
        .arg(
            Arg::with_name("output")
                .short("o")
                .long("output")
                .takes_value(true)
                .help("Output keys to specified dir"),
        )
        .get_matches();

    let num = match matches.value_of("num") {
        Some(nstr) => {
            if let Ok(num) = nstr.parse::<usize>() {
                num
            } else {
                println!("expected peer number");
                exit(-1);
            }
        }
        None => {
            println!("expected peer number");
            exit(-1);
        }
    };

    let (_, pkset, vec_skshare) = hs_data::threshold_sign_kit(num, (num << 1) / 3);
    //  output format:
    //
    //  num=4
    //  threshold=2
    //  sk_id=0
    //  sk_share=ABCDEFG
    //  pk_set=HIJKLMN

    let conts = vec_skshare.into_iter().map(|(sk_id, skshare)| {
        let cont = [
            format!("num={}", num),
            format!("th={}", (num << 1) / 3),
            format!("sk_id={}", sk_id),
            format!(
                "sk_share={}",
                serde_json::to_string(&SerdeSecret(skshare)).unwrap()
            ),
            format!("pk_set={}", serde_json::to_string(&pkset).unwrap()),
        ]
        .join("\n");
        (sk_id, cont)
    });

    if let Some(output_path) = matches.value_of("output") {
        let path = std::path::Path::new(output_path);
        if !path.is_dir() {
            println!("dir not exists");
            exit(-1);
        }

        for (sk_id, cont) in conts {
            match std::fs::File::create(path.join(format!("crypto-{}", sk_id))) {
                Ok(mut f) => {
                    f.write(cont.as_bytes()).unwrap();
                }
                Err(e) => {
                    println!("err:{:?}", e);
                    exit(-1);
                }
            }
        }
    } else {
        for (_, cont) in conts {
            println!("{}\n\n", cont);
        }
    }
}
