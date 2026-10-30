//! `--play FILE`: replays a stream saved with `--dump`, for development
//! without a sender.

use std::path::PathBuf;
use std::time::Duration;

use riflesso_airplay::{Codec, Event, VideoFrame};

use crate::{MirrorEvent, MirrorProxy};

pub fn spawn(path: PathBuf, mirror: MirrorProxy) {
    std::thread::spawn(move || {
        let data = match std::fs::read(&path) {
            Ok(data) => data,
            Err(e) => {
                tracing::error!("reading {}: {e}", path.display());
                return;
            }
        };
        // The first parameter set tells: a VPS (NAL type 32) for H.265, an SPS
        // (type 7) for H.264. Delimiters may come before it.
        let first = data
            .windows(5)
            .find(|w| w[..4] == [0, 0, 0, 1] && (w[4] & 0x1F == 7 || (w[4] >> 1) & 0x3F == 32))
            .map(|w| w[4]);
        let codec = if first.is_some_and(|b| (b >> 1) & 0x3F == 32) { Codec::Hevc } else { Codec::H264 };
        let units = split_access_units(&data, codec);
        tracing::info!("playing {} ({codec:?}, {} frames)", path.display(), units.len());
        for unit in units {
            let frame = VideoFrame { codec, annex_b: unit.to_vec(), keyframe: false, timestamp: 0 };
            if mirror.send_event(MirrorEvent::AirPlay(Event::VideoFrame(frame))).is_err() {
                return;
            }
            std::thread::sleep(Duration::from_millis(33));
        }
        tracing::info!("end of file");
    });
}

/// Splits an Annex-B stream into access units: each slice NAL unit together
/// with the parameter sets and SEI that precede it.
fn split_access_units(data: &[u8], codec: Codec) -> Vec<&[u8]> {
    let starts: Vec<usize> =
        data.windows(4).enumerate().filter(|(_, w)| w == &[0, 0, 0, 1]).map(|(i, _)| i).collect();
    let mut units = Vec::new();
    let mut unit_start: Option<usize> = None;
    for (n, &start) in starts.iter().enumerate() {
        let first = *unit_start.get_or_insert(start);
        let header = data.get(start + 4).copied().unwrap_or(0);
        let is_slice = match codec {
            Codec::H264 => matches!(header & 0x1F, 1 | 5),
            Codec::Hevc => (header >> 1) & 0x3F < 32,
        };
        if is_slice {
            let end = starts.get(n + 1).copied().unwrap_or(data.len());
            units.push(&data[first..end]);
            unit_start = None;
        }
    }
    units
}
