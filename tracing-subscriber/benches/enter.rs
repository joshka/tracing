use criterion::{criterion_group, criterion_main, Criterion};
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::prelude::*;

fn enter(c: &mut Criterion) {
    let mut group = c.benchmark_group("enter");
    let _subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .finish()
        .set_default();
    group.bench_function("enabled", |b| {
        let span = tracing::info_span!("foo");
        b.iter_with_large_drop(|| span.enter())
    });
    group.bench_function("disabled", |b| {
        let span = tracing::debug_span!("foo");
        b.iter_with_large_drop(|| span.enter())
    });
}

fn enter_exit(c: &mut Criterion) {
    let mut group = c.benchmark_group("enter_exit");
    let _subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .finish()
        .set_default();
    group.bench_function("enabled", |b| {
        let span = tracing::info_span!("foo");
        b.iter(|| span.enter())
    });
    group.bench_function("disabled", |b| {
        let span = tracing::debug_span!("foo");
        b.iter(|| span.enter())
    });
}

fn enter_many(c: &mut Criterion) {
    let mut group = c.benchmark_group("enter_many");
    let _subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .finish()
        .set_default();
    group.bench_function("enabled", |b| {
        let span1 = tracing::info_span!("span1");
        let _e1 = span1.enter();
        let span2 = tracing::info_span!("span2");
        let _e2 = span2.enter();
        let span3 = tracing::info_span!("span3");
        let _e3 = span3.enter();
        let span = tracing::info_span!("foo");
        b.iter_with_large_drop(|| span.enter())
    });
    group.bench_function("disabled", |b| {
        let span1 = tracing::info_span!("span1");
        let _e1 = span1.enter();
        let span2 = tracing::info_span!("span2");
        let _e2 = span2.enter();
        let span3 = tracing::info_span!("span3");
        let _e3 = span3.enter();
        let span = tracing::debug_span!("foo");
        b.iter_with_large_drop(|| span.enter())
    });
}

fn timed_enter_exit(c: &mut Criterion) {
    let mut group = c.benchmark_group("timed_enter_exit");

    // This is the control case for the layer's span-event checks. The span is
    // created outside the loop so the measurement covers only enter and exit.
    group.bench_function("span_events_disabled", |b| {
        let subscriber = tracing_subscriber::fmt()
            .with_writer(std::io::sink)
            .with_span_events(FmtSpan::NONE)
            .finish();
        let dispatch = tracing::Dispatch::new(subscriber);
        tracing::dispatcher::with_default(&dispatch, || {
            let span = tracing::info_span!("foo");
            b.iter(|| span.enter())
        });
    });

    // Closing a timed span reports its accumulated idle and busy durations.
    // Each outermost enter/exit transition must therefore sample the clock and
    // update the timing state, even though the close event is outside the loop.
    group.bench_function("timed_close", |b| {
        let subscriber = tracing_subscriber::fmt()
            .with_writer(std::io::sink)
            .with_span_events(FmtSpan::CLOSE)
            .finish();
        let dispatch = tracing::Dispatch::new(subscriber);
        tracing::dispatcher::with_default(&dispatch, || {
            let span = tracing::info_span!("foo");
            b.iter(|| span.enter())
        });
    });

    // Re-entering an already-entered span changes only its nesting depth. This
    // case guards against paying for a clock sample or duration update when no
    // idle/busy transition occurred.
    group.bench_function("timed_close_nested", |b| {
        let subscriber = tracing_subscriber::fmt()
            .with_writer(std::io::sink)
            .with_span_events(FmtSpan::CLOSE)
            .finish();
        let dispatch = tracing::Dispatch::new(subscriber);
        tracing::dispatcher::with_default(&dispatch, || {
            let span = tracing::info_span!("foo");
            let _outer = span.enter();
            b.iter(|| span.enter())
        });
    });

    // The span acquires timing state while CLOSE events are enabled, then the
    // layer is reconfigured. Its enter/exit callbacks must still maintain that
    // state in case CLOSE is enabled again before the span closes. Unlike the
    // disabled control case, this deliberately measures that retained work.
    group.bench_function("timing_disabled_after_span_created", |b| {
        let layer = tracing_subscriber::fmt::layer()
            .with_writer(std::io::sink)
            .with_span_events(FmtSpan::CLOSE);
        let (layer, reload_handle) = tracing_subscriber::reload::Layer::new(layer);
        let subscriber = tracing_subscriber::registry().with(layer);
        let dispatch = tracing::Dispatch::new(subscriber);
        tracing::dispatcher::with_default(&dispatch, || {
            let span = tracing::info_span!("foo");
            reload_handle
                .modify(|layer| layer.set_span_events(FmtSpan::NONE))
                .unwrap();
            b.iter(|| span.enter())
        });
    });

    // A timing extension elsewhere keeps the conservative lookup enabled, but
    // this span is created after CLOSE events are disabled and has no timing
    // state of its own. This isolates the empty extension-lookup cost from the
    // clock and accumulation work measured by the previous case.
    group.bench_function("timing_disabled_new_span", |b| {
        let layer = tracing_subscriber::fmt::layer()
            .with_writer(std::io::sink)
            .with_span_events(FmtSpan::CLOSE);
        let (layer, reload_handle) = tracing_subscriber::reload::Layer::new(layer);
        let subscriber = tracing_subscriber::registry().with(layer);
        let dispatch = tracing::Dispatch::new(subscriber);
        tracing::dispatcher::with_default(&dispatch, || {
            let _timed_span = tracing::info_span!("timed");
            reload_handle
                .modify(|layer| layer.set_span_events(FmtSpan::NONE))
                .unwrap();
            let span = tracing::info_span!("untimed");
            b.iter(|| span.enter())
        });
    });
}

criterion_group!(benches, enter, enter_exit, enter_many, timed_enter_exit);
criterion_main!(benches);
