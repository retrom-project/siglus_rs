//! Browser OMV playback keeps the decoder and the displayed frame, not a filmstrip.
//! Compressed input still uses the existing whole-file VFS. This module bounds
//! decoded video independently of duration; it does not claim range-based I/O.
use super::*;
use siglus_assets::omv::OmvFile;
use siglus_omv_decoder::{TheoraFile, TheoraVideoStream};

const DECODE_STEPS_PER_POLL: usize = 8;
const MAX_AUDIO_PCM_BYTES: usize = 64 * 1024 * 1024;

pub(super) struct BrowserOmvStream {
    source: MovieBytes,
    index: OmvFile,
    decoder: TheoraVideoStream<Cursor<MovieBytes>>,
    info: MovieInfo,
    frame: Option<(usize, Arc<RgbaImage>)>,
    pending: Option<(usize, Arc<RgbaImage>)>,
    next_frame_idx: usize,
    done: bool,
    position_ms: u64,
    pub(super) idle_frames: u32,
    pub(super) audio: Option<MovieAudio>,
}

impl BrowserOmvStream {
    pub(super) fn open(path: &Path, bytes: Vec<u8>) -> Result<Self> {
        let audio = read_audio(&bytes)?;
        let mut stream = Self::open_video(path, bytes)?;
        stream.audio = audio;
        Ok(stream)
    }

    pub(super) fn open_video(path: &Path, bytes: Vec<u8>) -> Result<Self> {
        let index = OmvFile::from_reader(Cursor::new(&bytes))?;
        let source: MovieBytes = bytes.into();
        let mut reader = Cursor::new(source.clone());
        reader.set_position(index.ogg_data_offset);
        let decoder = TheoraVideoStream::open(reader)?;
        let fps = if index.header.frame_time_us > 0 {
            1_000_000.0 / index.header.frame_time_us as f32
        } else {
            decoder.info().fps as f32
        };
        if !fps.is_finite() || fps <= 0.0 {
            bail!("OMV has no valid presentation frame rate");
        }
        let total_ms = index
            .packets
            .last()
            .map(|p| p.frame_time_end.max(0) as u64)
            .filter(|ms| *ms > 0)
            .or_else(|| {
                (index.header.packet_count_hint > 0).then(|| {
                    (index.header.packet_count_hint as f64 * 1000.0 / fps as f64).round() as u64
                })
            });
        let info = MovieInfo {
            path: path.to_path_buf(),
            width: Some(index.header.display_width),
            height: Some(index.header.display_height),
            fps: Some(fps),
            decoded_frames: (index.header.packet_count_hint > 0)
                .then_some(index.header.packet_count_hint as usize),
            audio_duration_ms: total_ms,
        };
        Ok(Self {
            source,
            index,
            decoder,
            info,
            frame: None,
            pending: None,
            next_frame_idx: 0,
            done: false,
            position_ms: 0,
            idle_frames: 0,
            audio: None,
        })
    }

    fn restart(&mut self) -> Result<()> {
        let mut reader = Cursor::new(self.source.clone());
        reader.set_position(self.index.ogg_data_offset);
        self.decoder = TheoraVideoStream::open(reader)?;
        self.frame = None;
        self.pending = None;
        self.next_frame_idx = 0;
        self.done = false;
        Ok(())
    }

    fn frame_time(&self, idx: usize) -> u64 {
        self.index
            .packets
            .get(idx)
            .map(|p| p.frame_time_start.max(0) as u64)
            .unwrap_or_else(|| (idx as f64 * 1000.0 / self.info.fps.unwrap() as f64).round() as u64)
    }

    fn make_frame(&self, packed: &[u8]) -> Arc<RgbaImage> {
        let v = self.decoder.info();
        let h = &self.index.header;
        let rgba = convert_omv_frame(
            packed,
            v.frame_width,
            v.frame_height,
            v.fmt,
            h.display_width as i32,
            h.display_height as i32,
            h.theora_type,
        );
        Arc::new(RgbaImage {
            width: h.display_width,
            height: h.display_height,
            center_x: 0,
            center_y: 0,
            rgba,
        })
    }

    pub(super) fn poll(
        &mut self,
        timer_ms: u64,
        loop_flag: bool,
    ) -> Result<Option<MovieStreamFrame>> {
        self.idle_frames = 0;
        let duration = self.info.duration_ms();
        let effective = if loop_flag {
            duration
                .filter(|ms| *ms > 0)
                .map(|ms| timer_ms % ms)
                .unwrap_or(timer_ms)
        } else {
            timer_ms
        };
        let mut desired = if self.index.packets.is_empty() {
            (effective as f64 * self.info.fps.unwrap() as f64 / 1000.0).floor() as usize
        } else {
            self.index
                .packets
                .partition_point(|p| effective > p.frame_time_end.max(0) as u64)
                .min(self.index.packets.len() - 1)
        };
        if let Some(count) = self.info.decoded_frames {
            desired = desired.min(count.saturating_sub(1));
        }
        // Compare against the last served position, not a previous unclamped
        // request: slow decoding must not look like an explicit rewind.
        let rewound = effective.saturating_add(100) < self.position_ms;
        if rewound {
            self.restart()?;
        }
        if self
            .pending
            .as_ref()
            .is_some_and(|(idx, _)| *idx <= desired)
        {
            self.frame = self.pending.take();
        }
        if omv_should_index_seek(
            &self.index,
            self.frame.as_ref().map(|(last, _)| *last).unwrap_or(0),
            desired,
        ) {
            let point = self.index.seek_point_for_frame(desired)?;
            if let Some(packed) = self.decoder.seek_to_indexed_frame(
                point.file_offset,
                point.key_page_file_offset,
                point.first_packet_no,
                point.key_frame_packet_no,
                point.target_packet_no,
            )? {
                self.frame = Some((desired, self.make_frame(&packed)));
            }
            self.pending = None;
            self.next_frame_idx = desired.saturating_add(1);
            self.done = false;
        }
        let mut decoded_now = false;
        let mut selected = None;
        for _ in 0..DECODE_STEPS_PER_POLL {
            if self.done || self.pending.is_some() || self.next_frame_idx > desired {
                break;
            }
            match self.decoder.read_video_frame()? {
                Some(packed) => {
                    self.next_frame_idx = self.decoder.next_video_packet_index();
                    let idx = self.next_frame_idx.saturating_sub(1);
                    if idx > desired && self.frame.is_some() {
                        self.pending = Some((idx, self.make_frame(&packed)));
                    } else {
                        // Intermediate reference frames need decoding, but
                        // only the frame we will display needs RGBA conversion.
                        selected = Some((idx, packed));
                    }
                    decoded_now = true;
                }
                None => {
                    self.done = true;
                    self.next_frame_idx = self.decoder.next_video_packet_index();
                    self.info.decoded_frames = Some(self.next_frame_idx);
                    if self.info.audio_duration_ms.is_none() {
                        self.info.audio_duration_ms = Some(self.frame_time(self.next_frame_idx));
                    }
                }
            }
        }
        if let Some((idx, packed)) = selected {
            self.frame = Some((idx, self.make_frame(&packed)));
        }
        let Some((idx, frame)) = self.frame.as_ref() else {
            bail!("OMV contains no decoded video frames");
        };
        let catching_up = !self.done && self.pending.is_none() && self.next_frame_idx <= desired;
        self.position_ms = if catching_up {
            self.frame_time(*idx)
        } else {
            effective
        };
        Ok(Some(MovieStreamFrame {
            frame: frame.clone(),
            frame_idx: *idx,
            fps: self.info.fps,
            total_ms: self.info.duration_ms(),
            audio: self.audio.clone(),
            audio_ready: true,
            decoded_now,
            frame_idx_is_decode_order: false,
            clamped_timer_ms: catching_up.then_some(self.position_ms),
            source_size: self.info.width.zip(self.info.height),
        }))
    }

    pub(super) fn retained_frames(&self) -> impl Iterator<Item = &Arc<RgbaImage>> {
        self.frame
            .iter()
            .chain(self.pending.iter())
            .map(|(_, frame)| frame)
    }
}

// Keep existing embedded soundtracks. Convert chunks directly to i16 and
// reject an over-limit static track instead of allocating unbounded PCM.
fn read_audio(bytes: &[u8]) -> Result<Option<MovieAudio>> {
    let mut tf = TheoraFile::open_from_memory(extract_ogg_from_bytes(bytes)?)?;
    let Some((channels, sample_rate)) = tf.audio_info() else {
        return Ok(None);
    };
    let mut samples = Vec::new();
    let mut buf = vec![0.0; 4096 * channels as usize];
    loop {
        let n = tf.read_audio_samples(&mut buf)?;
        if n == 0 {
            break;
        }
        if samples.len() + n > MAX_AUDIO_PCM_BYTES / 2 {
            bail!("OMV static audio exceeds {} bytes", MAX_AUDIO_PCM_BYTES);
        }
        samples.extend(buf[..n].iter().map(|s| {
            (if s.is_nan() { -1.0 } else { s.clamp(-1.0, 1.0) } * 32767.0).round() as i16
        }));
    }
    if samples.is_empty() {
        return Ok(None);
    }
    let duration =
        (samples.len() as f64 * 1000.0 / channels as f64 / sample_rate as f64).round() as u64;
    Ok(Some(MovieAudio {
        samples: Arc::new(samples),
        mpeg_stream: None,
        wmv_stream: None,
        channels: channels as u16,
        sample_rate: sample_rate as u32,
        start_ms: 0,
        duration_ms: Some(duration),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream() -> BrowserOmvStream {
        BrowserOmvStream::open(
            Path::new("counter.omv"),
            include_bytes!("fixtures/counter.omv").to_vec(),
        )
        .unwrap()
    }

    #[test]
    fn preview_decodes_one_frame_and_playback_retention_does_not_grow() {
        let mut stream = stream();
        let first = stream.poll(0, false).unwrap().unwrap();
        assert_eq!(stream.next_frame_idx, 1);
        let first_pixels = first.frame.rgba.clone();
        drop(first);
        for ms in (34..=3060).step_by(34) {
            let frame = stream.poll(ms, false).unwrap().unwrap();
            assert_eq!(frame.frame.rgba.len(), 64 * 48 * 4);
            assert!(stream.retained_frames().count() <= 2);
            assert!(
                stream
                    .retained_frames()
                    .map(|f| f.rgba.len())
                    .sum::<usize>()
                    <= 2 * 64 * 48 * 4
            );
        }
        assert!(stream.done);
        assert_eq!(stream.info.decoded_frames, Some(90));
        assert_eq!(stream.info.duration_ms(), Some(3000));
        let looped = stream.poll(3001, true).unwrap().unwrap();
        assert_eq!(looped.frame_idx, 0);
        assert_eq!(looped.frame.rgba, first_pixels);
        assert_eq!(stream.next_frame_idx, 1);
    }

    #[test]
    fn forward_jumps_have_a_work_budget_and_clamping_does_not_restart() {
        let mut stream = stream();
        let frame = stream.poll(2000, false).unwrap().unwrap();
        assert!(frame.frame_idx < 60);
        let previous_idx = frame.frame_idx;
        let clamped = frame.clamped_timer_ms.expect("decoder is catching up");
        let frame = stream.poll(clamped + 100, false).unwrap().unwrap();
        assert!(frame.frame_idx > previous_idx);
    }

    #[test]
    fn duplicate_packets_hold_pixels_until_the_next_presentation_time() {
        let mut stream = BrowserOmvStream::open(
            Path::new("duplicated.omv"),
            include_bytes!("fixtures/duplicated.omv").to_vec(),
        )
        .unwrap();
        let first = stream.poll(0, false).unwrap().unwrap();
        let duplicate = stream.poll(34, false).unwrap().unwrap();
        assert_eq!(duplicate.frame_idx, 0);
        assert_eq!(duplicate.frame.rgba, first.frame.rgba);
        assert_eq!(duplicate.clamped_timer_ms, None);
        let next = stream.poll(67, false).unwrap().unwrap();
        assert_eq!(next.frame_idx, 2);
        assert_ne!(next.frame.rgba, first.frame.rgba);
        for ms in (100..=6100).step_by(34) {
            stream.poll(ms, false).unwrap();
        }
        assert_eq!(stream.info.decoded_frames, Some(180));
        assert_eq!(stream.info.duration_ms(), Some(6000));
    }
}
