//! `victory warm`: the title's shaders translated into the shader cache
//! before a run (see docs/precompilacao-shaders.md).
//!
//! A run translates each shader when the title creates a pipeline with it,
//! on the pipeline workers, while the title waits up to its budget per
//! frame (past it the draw is dropped). The translation is a function of
//! the container's bytes and of this build alone (the cache key:
//! `victory_gpu::shader_cache`), so every container found in the title's
//! files can be translated ahead of time into the same entries. The run
//! then reads them back instead.
//!
//! Compute PSOs are compiled into the runtime's pipeline library too, without
//! dispatching them. Graphics PSOs still need the state supplied during play.
//! Based on DexNom/VIctory's portable archive preload workflow.

use std::path::PathBuf;

#[cfg(windows)]
use anyhow::Context;
use anyhow::Result;

#[cfg(windows)]
pub fn run(paths: Vec<PathBuf>, list: bool, jobs: Option<usize>, pipelines: bool) -> Result<()> {
    use std::collections::{BTreeMap, HashSet};
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
    use std::time::Instant;

    use victory_hle::modules::d3d12x;
    use victory_shader::Container;

    use crate::shader::{MAX_FILE, sources};

    let paths = crate::default_paths(paths)?;
    let cfg = victory_core::Config::load()?;
    let dir = cfg.shader_cache()?;
    println!("shader cache {}", dir.display());
    println!(
        "note         translations match this translator, GPU capabilities and VICTORY_* settings;"
    );
    println!("             warm with the same build and settings you run the title with");
    d3d12x::use_shader_cache(dir);
    println!(
        "pipelines    {}",
        if pipelines {
            "compute PSOs compiled, never dispatched"
        } else {
            "translation only"
        }
    );

    let threads =
        jobs.unwrap_or_else(|| std::thread::available_parallelism().map_or(4, |n| n.get().min(4)));
    anyhow::ensure!(threads > 0, "--jobs must be greater than zero");
    let seen = Mutex::new(HashSet::new());
    let failures: Mutex<BTreeMap<String, (usize, String)>> = Mutex::new(BTreeMap::new());
    let (found, cached, translated, failed) = (
        AtomicUsize::new(0),
        AtomicUsize::new(0),
        AtomicUsize::new(0),
        AtomicUsize::new(0),
    );
    let started = Instant::now();
    let (made, unmade) = (AtomicUsize::new(0), AtomicUsize::new(0));
    for root in &paths {
        for entry in walkdir::WalkDir::new(root) {
            let entry = entry.with_context(|| format!("scanning {}", root.display()))?;
            if !entry.file_type().is_file() {
                continue;
            }
            if entry.metadata()?.len() > MAX_FILE {
                tracing::warn!(path = %entry.path().display(), "shader scan skipped a file larger than 2 GiB");
                continue;
            }
            let data = std::fs::read(entry.path())
                .with_context(|| format!("reading {}", entry.path().display()))?;
            let jobs = sources(&data);
            if jobs.is_empty() {
                continue;
            }
            let path = entry.path();
            let next = AtomicUsize::new(0);
            std::thread::scope(|s| {
                for _ in 0..threads.min(jobs.len()) {
                    s.spawn(|| {
                        while let Some(job) = jobs.get(next.fetch_add(1, Relaxed)) {
                            let Some(bytes) = job.bytes() else {
                                tracing::warn!(path = %path.display(), record = %job.label(), "corrupt shader record");
                                continue;
                            };
                            // The bytes a title passes: this container only,
                            // not the rest of the file after it.
                            let container = match Container::bytes_of(&bytes) {
                                Ok(container) => container,
                                Err(e) => {
                                    tracing::warn!(path = %path.display(), error = %e, "invalid shader container");
                                    continue;
                                }
                            };
                            let new = seen
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .insert(blake3::hash(container));
                            if !new {
                                continue;
                            }
                            let n = found.fetch_add(1, Relaxed) + 1;
                            let warmed = match d3d12x::warm_shader(container) {
                                Ok(false) => {
                                    cached.fetch_add(1, Relaxed);
                                    true
                                }
                                Ok(true) => {
                                    translated.fetch_add(1, Relaxed);
                                    true
                                }
                                Err(e) => {
                                    failed.fetch_add(1, Relaxed);
                                    let example = format!("{}{}", path.display(), job.label());
                                    if list {
                                        println!("does not translate: {example}: {e}");
                                    }
                                    let mut f = failures.lock().unwrap_or_else(|e| e.into_inner());
                                    f.entry(e).or_insert((0, example)).0 += 1;
                                    false
                                }
                            };
                            if warmed && pipelines {
                                match d3d12x::warm_compute_pipeline(container) {
                                    Ok(Some(true)) => { made.fetch_add(1, Relaxed); }
                                    Ok(Some(false)) => { unmade.fetch_add(1, Relaxed); }
                                    Ok(None) => {}
                                    Err(e) => {
                                        unmade.fetch_add(1, Relaxed);
                                        tracing::warn!(path = %path.display(), error = %e, "compute pipeline warm-up failed");
                                    }
                                }
                            }
                            if n.is_multiple_of(2000) {
                                println!("{:.0} s  {n} distinct containers scanned", started.elapsed().as_secs_f64());
                            }
                        }
                    });
                }
            });
        }
    }
    if pipelines && found.load(Relaxed) > 0 {
        d3d12x::save_pipeline_cache();
    }
    let (found, cached, translated, failed) = (
        found.into_inner(),
        cached.into_inner(),
        translated.into_inner(),
        failed.into_inner(),
    );
    println!(
        "containers   {found} distinct: {translated} translated now, {cached} already cached, \
         {failed} do not translate ({:.1} s)",
        started.elapsed().as_secs_f64()
    );
    let mut failures: Vec<_> = failures
        .into_inner()
        .unwrap_or_else(|e| e.into_inner())
        .into_iter()
        .collect();
    failures.sort_by_key(|(_, (n, _))| std::cmp::Reverse(*n));
    for (reason, (n, example)) in failures.iter().take(20) {
        println!("  {n:>7}  {reason}  (e.g. {example})");
    }
    println!(
        "warm         {} translated, {failed} refused; {} compute pipelines, {} refused; {:.1} s",
        translated + cached,
        made.into_inner(),
        unmade.into_inner(),
        started.elapsed().as_secs_f64()
    );
    anyhow::ensure!(
        found > 0,
        "no shader containers found in the selected paths"
    );
    anyhow::ensure!(
        translated + cached > 0,
        "no shaders successfully translated; see the refusal totals"
    );
    Ok(())
}

#[cfg(not(windows))]
pub fn run(_: Vec<PathBuf>, _: bool, _: Option<usize>, _: bool) -> Result<()> {
    anyhow::bail!("warming the shader cache requires Windows (the host D3D12 device)")
}
