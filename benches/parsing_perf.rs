use std::time::Duration;
use criterion::{criterion_group, criterion_main, Criterion, black_box};
use haiku_common::metadata::ShmMetadata;
use haiku_fh::parsing::parsing_fast::StreamingParser;


fn custom_criterion() -> Criterion {
    Criterion::default()
        .measurement_time(Duration::from_secs(30))
        .sample_size(1000)
        .warm_up_time(Duration::from_secs(5))
        .noise_threshold(0.05)
}

fn benchmark_fast_parser(c: &mut Criterion) {
    let metadata =
        ShmMetadata::load_from_file("/home/gitgud/haikutrading/shm/test/rust_integration.json");

    let fast_parser = StreamingParser::new(metadata.unwrap().clone_instrument_index());
    // let json_data = r#"{"jsonrpc":"2.0","method":"subscription","params":{"channel":"trades.ETH-PERPETUAL.raw","data":[{"timestamp":1753469786907,"price":3652.7,"amount":13830.0,"direction":"sell","index_price":3652.65,"instrument_name":"ETH-PERPETUAL","trade_seq":187471858,"mark_price":3652.16,"tick_direction":0,"trade_id":"ETH-259727150","contracts":13830.0},{"timestamp":1753469786907,"price":3652.7,"amount":29.0,"direction":"sell","index_price":3652.65,"instrument_name":"ETH-PERPETUAL","trade_seq":187471859,"mark_price":3652.16,"tick_direction":1,"trade_id":"ETH-259727151","contracts":29.0}]}}"#;
    let json_data = r#"{"jsonrpc":"2.0","method":"subscription","params":{"channel":"trades.ETH-PERPETUAL.raw","data":[{"timestamp":1753469821143,"price":3653.4,"amount":139.0,"direction":"buy","index_price":3654.33,"instrument_name":"ETH-PERPETUAL","trade_seq":187471866,"mark_price":3653.79,"tick_direction":0,"trade_id":"ETH-259727165","contracts":139.0}]}}"#;


    c.bench_function("parse_fast", |b| {
        b.iter(|| {
            let trades = fast_parser.parse_fast_new(json_data.as_bytes());

        })
    });
}


criterion_group! {
    name = benches;
    config = custom_criterion();
    targets = benchmark_fast_parser
}

criterion_main!(benches);