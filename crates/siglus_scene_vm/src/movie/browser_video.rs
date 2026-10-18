//! Incremental browser WMV / MPEG playback. Compressed VFS input is retained,
//! but decoded pictures are bounded by codec reordering, never movie duration.
use super::*;

const DECODE_STEPS: usize = 8;
const INPUT_CHUNK: usize = 2048;
const MAX_AUDIO_PCM_BYTES: usize = 64 * 1024 * 1024;

enum Picture {
    Wmv(wmv_decoder::YuvFrame),
    Mpeg(Arc<na_mpeg2_decoder::Frame>),
}

impl Picture {
    fn rgba(&self) -> Arc<RgbaImage> {
        Arc::new(match self {
            Self::Wmv(frame) => wmv_yuv_frame_to_rgba(frame),
            Self::Mpeg(frame) => {
                let width = frame.width as u32;
                let height = frame.height as u32;
                let mut rgba = vec![0; width as usize * height as usize * 4];
                na_mpeg2_decoder::frame_to_rgba_bt601_limited(frame, &mut rgba);
                RgbaImage {
                    width,
                    height,
                    center_x: 0,
                    center_y: 0,
                    rgba,
                }
            }
        })
    }

    fn bytes(&self) -> usize {
        match self {
            Self::Wmv(f) => f.y.len() + f.cb.len() + f.cr.len(),
            Self::Mpeg(f) => f.data_y.len() + f.data_u.len() + f.data_v.len(),
        }
    }
}

struct TimedPicture {
    pts: u64,
    picture: Picture,
}

/// Pull one presentation picture at a time, even for a PES containing many
/// pictures. Feeding a whole file to the old Vec-returning decoder was the
/// second full-film cache, underneath MovieAsset's RGBA cache.
struct MpegPull {
    source: MovieBytes,
    offset: usize,
    demux: na_mpeg2_decoder::Demuxer,
    decoder: na_mpeg2_decoder::Decoder,
    packets: VecDeque<na_mpeg2_decoder::Packet>,
    pts: Option<i64>,
    sent_end: bool,
    flushed: bool,
    tail: VecDeque<Arc<na_mpeg2_decoder::Frame>>,
    frames: usize,
    fps: f32,
}

impl MpegPull {
    fn new(source: MovieBytes, fps: f32) -> Self {
        Self {
            source,
            fps,
            offset: 0,
            demux: na_mpeg2_decoder::Demuxer::new_auto(),
            decoder: na_mpeg2_decoder::Decoder::new(),
            packets: VecDeque::new(),
            pts: None,
            sent_end: false,
            flushed: false,
            tail: VecDeque::new(),
            frames: 0,
        }
    }

    fn next(&mut self) -> Result<Option<TimedPicture>> {
        loop {
            let frame = if let Some(frame) = self.tail.pop_front() {
                Some(frame)
            } else {
                self.decoder.decode_shared_limited(&[], self.pts, 1)?.pop()
            };
            if let Some(frame) = frame {
                let pts = (self.frames as f64 * 1000.0 / self.fps as f64).round() as u64;
                self.frames += 1;
                return Ok(Some(TimedPicture {
                    pts,
                    picture: Picture::Mpeg(frame),
                }));
            }
            if let Some(packet) = self.packets.pop_front() {
                if packet.stream_type == na_mpeg2_decoder::StreamType::MpegVideo {
                    self.pts = packet.pts_90k;
                    // A zero limit appends input without decoding any picture.
                    self.decoder
                        .decode_shared_limited(&packet.data, self.pts, 0)?;
                }
                continue;
            }
            if self.offset < self.source.len() {
                let end = (self.offset + INPUT_CHUNK).min(self.source.len());
                self.packets
                    .extend(self.demux.push(&self.source[self.offset..end], None));
                self.offset = end;
                continue;
            }
            if !self.sent_end {
                // Terminate the final slice before flushing delayed references.
                self.decoder
                    .decode_shared_limited(&[0, 0, 1, 0xb7, 0, 0, 1, 0xb7], self.pts, 0)?;
                self.sent_end = true;
                continue;
            }
            if !self.flushed {
                self.tail.extend(self.decoder.flush_shared()?);
                self.flushed = true;
                continue;
            }
            return Ok(None);
        }
    }
}

enum Decoder {
    Wmv(wmv_decoder::AsfWmv2Decoder<Cursor<MovieBytes>>),
    Mpeg(MpegPull),
}

impl Decoder {
    fn next(&mut self) -> Result<Option<TimedPicture>> {
        match self {
            Self::Wmv(d) => Ok(d.next_frame()?.map(|f| TimedPicture {
                pts: f.pts_ms as u64,
                picture: Picture::Wmv(f.frame),
            })),
            Self::Mpeg(d) => d.next(),
        }
    }
}

pub(super) struct BrowserVideoStream {
    source: MovieBytes,
    decoder: Decoder,
    is_wmv: bool,
    reorder: usize,
    queued: Vec<TimedPicture>,
    eof: bool,
    origin: Option<u64>,
    presented: usize,
    frame: Option<(usize, u64, Arc<RgbaImage>)>,
    position_ms: u64,
    info: MovieInfo,
    pub(super) audio: Option<MovieAudio>,
    pub(super) idle_frames: u32,
}

impl BrowserVideoStream {
    pub(super) fn open(path: &Path, bytes: Vec<u8>) -> Result<Self> {
        let mut stream = Self::open_video(path, bytes)?;
        stream.fill_reorder()?;
        let origin = stream.queued.first().map(|f| f.pts).unwrap_or(0);
        stream.audio = if stream.is_wmv {
            decode_wmv_audio_full_from_reader(
                Cursor::new(stream.source.clone()),
                &AtomicBool::new(false),
                origin,
            )?
        } else {
            read_mpeg_audio(path, &stream.source)?
        };
        Ok(stream)
    }

    pub(super) fn open_video(path: &Path, bytes: Vec<u8>) -> Result<Self> {
        let source: MovieBytes = bytes.into();
        let is_wmv = path
            .extension()
            .and_then(|s| s.to_str())
            .is_some_and(|s| is_wmv_extension(&s.to_ascii_lowercase()));
        let (decoder, reorder, width, height, fps, duration) = if is_wmv {
            let decoder = wmv_decoder::AsfWmv2Decoder::open(Cursor::new(source.clone()))?;
            let v = decoder.video_stream_info();
            let reorder = match &v.codec_four_cc {
                b"WMV3" => {
                    wmv_decoder::vc1::SequenceHeader::parse(&v.extra_data)?.max_b_frames as usize
                }
                b"WVC1" => 7, // VC-1 advanced profile: use the bounded maximum.
                _ => 0,
            };
            let (width, height, duration) = (v.width, v.height, decoder.duration_ms());
            (
                Decoder::Wmv(decoder),
                reorder.min(7),
                width,
                height,
                None,
                duration,
            )
        } else {
            let header = siglus_assets::mpeg2::find_sequence_header(
                &source[..source.len().min(MPEG2_HEADER_PROBE_BYTES)],
            )
            .ok_or_else(|| anyhow!("MPEG sequence header missing: {}", path.display()))?;
            let fps = siglus_assets::mpeg2::fps_from_frame_rate_code(header.frame_rate_code)
                .filter(|f| *f > 0.0)
                .ok_or_else(|| anyhow!("MPEG has no frame rate"))?;
            // Compressed-only metadata pass: no decoder, YUV or RGBA allocation.
            let frames = mpeg_picture_count(&source);
            let duration = Some((frames as f64 * 1000.0 / fps as f64).round() as u64);
            (
                Decoder::Mpeg(MpegPull::new(source.clone(), fps)),
                0,
                header.width as u32,
                header.height as u32,
                Some(fps),
                duration,
            )
        };
        Ok(Self {
            source,
            decoder,
            is_wmv,
            reorder,
            queued: Vec::new(),
            eof: false,
            origin: None,
            presented: 0,
            frame: None,
            position_ms: 0,
            audio: None,
            idle_frames: 0,
            info: MovieInfo {
                path: path.to_path_buf(),
                width: Some(width),
                height: Some(height),
                fps,
                decoded_frames: None,
                audio_duration_ms: duration,
            },
        })
    }

    fn fill_reorder(&mut self) -> Result<()> {
        // At most max_b_frames + 1 YUV pictures. One more picture can be in
        // the caller's current selection; only that selection becomes RGBA.
        while !self.eof && self.queued.len() <= self.reorder {
            match self.decoder.next()? {
                Some(frame) => self.queued.push(frame),
                None => self.eof = true,
            }
        }
        self.queued.sort_by_key(|f| f.pts);
        Ok(())
    }

    fn restart(&mut self) -> Result<()> {
        self.decoder = if self.is_wmv {
            Decoder::Wmv(wmv_decoder::AsfWmv2Decoder::open(Cursor::new(
                self.source.clone(),
            ))?)
        } else {
            Decoder::Mpeg(MpegPull::new(self.source.clone(), self.info.fps.unwrap()))
        };
        self.queued.clear();
        self.eof = false;
        self.presented = 0;
        self.frame = None;
        Ok(())
    }

    pub(super) fn poll(
        &mut self,
        timer_ms: u64,
        loop_flag: bool,
    ) -> Result<Option<MovieStreamFrame>> {
        self.idle_frames = 0;
        self.fill_reorder()?;
        if self.origin.is_none() {
            self.origin = self.queued.first().map(|f| f.pts);
            if self.is_wmv {
                self.info.audio_duration_ms = self
                    .info
                    .audio_duration_ms
                    .and_then(|d| wmv_rebase_duration_ms(d, self.origin.unwrap_or(0)));
            }
        }
        let mut total_ms = match (
            self.info.duration_ms(),
            self.audio.as_ref().map(MovieAudio::end_ms),
        ) {
            (Some(v), Some(a)) => Some(v.max(a)),
            (v, a) => v.or(a),
        };
        let effective = if loop_flag {
            total_ms
                .filter(|d| *d > 0)
                .map(|d| timer_ms % d)
                .unwrap_or(timer_ms)
        } else {
            timer_ms
        };
        if effective.saturating_add(100) < self.position_ms {
            self.restart()?;
            self.fill_reorder()?;
        }
        let origin = self.origin.unwrap_or(0);
        let mut selected = None;
        for _ in 0..DECODE_STEPS {
            if self
                .queued
                .first()
                .is_none_or(|f| f.pts > origin.saturating_add(effective))
            {
                break;
            }
            let picture = self.queued.remove(0);
            let idx = self.presented;
            self.presented += 1;
            selected = Some((idx, picture));
            self.fill_reorder()?;
        }
        let decoded_now = selected.is_some();
        if let Some((idx, picture)) = selected {
            let pts = picture.pts.saturating_sub(origin);
            if self.is_wmv
                && let Some((_, previous, _)) = self.frame.as_ref()
            {
                let delta = pts.saturating_sub(*previous);
                let count = idx.saturating_sub(self.frame.as_ref().unwrap().0);
                if count > 0 && delta > 0 {
                    self.info.fps = Some(1000.0 * count as f32 / delta as f32);
                }
            }
            self.frame = Some((idx, pts, picture.picture.rgba()));
        }
        if self.eof && self.queued.is_empty() {
            self.info.decoded_frames = Some(self.presented);
            if self.info.audio_duration_ms.is_none()
                && let Some((_, pts, _)) = self.frame.as_ref()
            {
                let step = self
                    .info
                    .fps
                    .filter(|f| *f > 0.0)
                    .map(|f| (1000.0 / f).round() as u64)
                    .unwrap_or(33);
                self.info.audio_duration_ms = Some(pts.saturating_add(step));
                let video = self.info.audio_duration_ms.unwrap();
                total_ms = Some(
                    self.audio
                        .as_ref()
                        .map(MovieAudio::end_ms)
                        .unwrap_or(0)
                        .max(video),
                );
            }
        }
        let Some((idx, pts, frame)) = self.frame.as_ref() else {
            if self.eof {
                bail!(
                    "movie contains no video pictures: {}",
                    self.info.path.display()
                );
            }
            return Ok(None);
        };
        let catching_up = self
            .queued
            .first()
            .is_some_and(|f| f.pts <= origin.saturating_add(effective));
        let clamped_timer_ms = catching_up.then_some(*pts);
        self.position_ms = clamped_timer_ms.unwrap_or(effective);
        Ok(Some(MovieStreamFrame {
            frame: frame.clone(),
            frame_idx: *idx,
            fps: self.info.fps,
            total_ms,
            audio: self.audio.clone(),
            audio_ready: true,
            decoded_now,
            frame_idx_is_decode_order: false,
            clamped_timer_ms,
            source_size: self.info.width.zip(self.info.height),
        }))
    }

    pub(super) fn frame(&self) -> Option<&Arc<RgbaImage>> {
        self.frame.as_ref().map(|f| &f.2)
    }
    pub(super) fn queued_bytes(&self) -> usize {
        self.queued.iter().map(|f| f.picture.bytes()).sum()
    }
    pub(super) fn queued_frames(&self) -> usize {
        self.queued.len()
    }
}

fn mpeg_picture_count(source: &[u8]) -> usize {
    let mut demux = na_mpeg2_decoder::Demuxer::new_auto();
    let mut tail = Vec::new();
    let mut count = 0;
    for chunk in source.chunks(INPUT_CHUNK) {
        for packet in demux.push(chunk, None) {
            if packet.stream_type != na_mpeg2_decoder::StreamType::MpegVideo {
                continue;
            }
            tail.extend_from_slice(&packet.data);
            count += tail.windows(4).filter(|w| *w == [0, 0, 1, 0]).count();
            if tail.len() > 3 {
                tail.drain(..tail.len() - 3);
            }
        }
    }
    count
}

fn read_mpeg_audio(path: &Path, source: &[u8]) -> Result<Option<MovieAudio>> {
    let mut pipeline = na_mpeg2_decoder::MpegAudioPipeline::new();
    let mut samples = Vec::new();
    let mut format = None;
    let mut failure = None;
    let mut first_pts = None;
    for chunk in source.chunks(INPUT_CHUNK) {
        pipeline.push_with(chunk, None, |a| {
            if failure.is_some() {
                return;
            }
            let current = (a.channels, a.sample_rate);
            if format.is_some_and(|f| f != current) {
                failure = Some("MPEG audio format changes during playback");
                return;
            }
            format = Some(current);
            first_pts.get_or_insert(a.pts_ms);
            if samples.len().saturating_add(a.samples.len()) > MAX_AUDIO_PCM_BYTES / 2 {
                failure = Some("MPEG static audio exceeds 64 MiB");
                return;
            }
            samples.extend(a.samples.into_iter().map(f32_to_i16_sample));
        })?;
        if let Some(message) = failure {
            bail!("{message}: {}", path.display());
        }
    }
    let Some((channels, sample_rate)) = format else {
        return Ok(None);
    };
    // Match the native MPEG origin: the earlier of the first video/audio PTS.
    let start_ms = first_pts
        .zip(pipeline.first_video_pts_ms())
        .map(|(audio, video)| audio.saturating_sub(video).max(0) as u64)
        .unwrap_or(0);
    let duration_ms = Some(samples.len() as u64 * 1000 / channels as u64 / sample_rate as u64);
    Ok(Some(MovieAudio {
        samples: Arc::new(samples),
        mpeg_stream: None,
        wmv_stream: None,
        channels,
        sample_rate,
        start_ms,
        duration_ms,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_asf_duration_is_discovered_at_eof_and_can_loop() {
        let mut stream = BrowserVideoStream::open_video(
            Path::new("no-duration.wmv"),
            include_bytes!("fixtures/no-duration.wmv").to_vec(),
        )
        .unwrap();
        let first = stream.poll(0, false).unwrap().unwrap();
        assert!(first.total_ms.is_none());
        let mut last = first.clone();
        for ms in (34..=3100).step_by(34) {
            last = stream.poll(ms, false).unwrap().unwrap();
        }
        assert_eq!(stream.presented, 90);
        assert!((2990..=3010).contains(&last.total_ms.unwrap()));
        let looped = stream.poll(last.total_ms.unwrap(), true).unwrap().unwrap();
        assert_eq!(looped.frame_idx, 0);
        assert_eq!(looped.frame.rgba, first.frame.rgba);
    }

    #[test]
    fn wmv_and_b_frame_mpeg_keep_bounded_pictures_and_replay_exactly() {
        for (name, bytes) in [
            (
                "counter.wmv",
                include_bytes!("fixtures/counter.wmv").as_slice(),
            ),
            (
                "counter.mpg",
                include_bytes!("fixtures/counter.mpg").as_slice(),
            ),
        ] {
            let mut stream = BrowserVideoStream::open(Path::new(name), bytes.to_vec()).unwrap();
            let first = stream.poll(0, false).unwrap().unwrap();
            let first_pixels = first.frame.rgba.clone();
            assert_eq!(
                stream.presented, 1,
                "preview must not decode the entire movie"
            );
            let duration = first.total_ms.unwrap();
            let mut distinct = HashSet::new();
            for ms in (0..duration + 100).step_by(34) {
                let f = stream.poll(ms, false).unwrap().unwrap();
                assert!(f.clamped_timer_ms.is_none());
                if name.ends_with("mpg") {
                    assert_eq!(f.fps, Some(30.0));
                }
                distinct.insert(f.frame.rgba.clone());
                assert!(stream.queued_frames() <= 8);
                assert!(stream.queued_bytes() <= 8 * 64 * 48 * 3 / 2);
                assert_eq!(stream.frame().unwrap().rgba.len(), 64 * 48 * 4);
            }
            assert!(stream.eof);
            assert_eq!(
                stream.presented, 90,
                "every picture must survive PES/EOF boundaries"
            );
            assert!(distinct.len() > 10);
            let looped = stream.poll(duration, true).unwrap().unwrap();
            assert_eq!(looped.frame_idx, 0);
            assert_eq!(looped.frame.rgba, first_pixels);
            if name.ends_with("mpg") {
                let audio = stream
                    .audio
                    .as_ref()
                    .expect("embedded MPEG audio must be retained");
                assert_eq!(audio.sample_rate, 32000);
                assert_eq!(audio.channels, 1);
                assert!(audio.samples.iter().any(|s| *s != 0));
            }
        }
    }

    #[test]
    fn large_timer_jumps_yield_and_resume_without_restarting() {
        for (name, bytes) in [
            (
                "counter.wmv",
                include_bytes!("fixtures/counter.wmv").as_slice(),
            ),
            (
                "counter.mpg",
                include_bytes!("fixtures/counter.mpg").as_slice(),
            ),
        ] {
            let mut stream =
                BrowserVideoStream::open_video(Path::new(name), bytes.to_vec()).unwrap();
            let f = stream.poll(2000, false).unwrap().unwrap();
            assert!(f.frame_idx < 60);
            let next = stream
                .poll(f.clamped_timer_ms.unwrap() + 100, false)
                .unwrap()
                .unwrap();
            assert!(next.frame_idx > f.frame_idx);
        }
    }
}
