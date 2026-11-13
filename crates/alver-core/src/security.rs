//! Hard limits for hostile input: avatar decode budgets, received filenames, and
//! per-peer rate limiting.
//!
//! Everything here is pure and synchronous so it can be tested — and property-tested —
//! without touching the filesystem, an image decoder, or a socket.

use std::collections::HashMap;

/// Longest accepted received path component in bytes.
pub const MAX_FILE_NAME_BYTES: usize = 255;
/// Longest accepted path-like received name in bytes.
pub const MAX_PATH_BYTES: usize = 1_024;
/// Largest avatar payload we will ever buffer.
pub const MAX_AVATAR_BYTES: usize = 512 * 1024;
/// Largest accepted avatar edge, before any decoding happens.
pub const MAX_AVATAR_DIMENSION: u32 = 1_024;
/// Smallest sane avatar edge; guards against 1x1 "decompression bomb" headers.
pub const MIN_AVATAR_DIMENSION: u32 = 1;
/// Largest accepted avatar pixel count (1024x1024).
pub const MAX_AVATAR_PIXELS: u64 = 1_024 * 1_024;
/// Avatar formats we will accept from a peer.
pub const ACCEPTED_AVATAR_MEDIA_TYPES: &[&str] =
    &["image/png", "image/jpeg", "image/gif", "image/webp"];

/// Why a piece of hostile input was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SecurityError {
    /// The value was empty.
    Empty,
    /// The name was an absolute path.
    AbsolutePath,
    /// The name walked out of the receive directory with `..`.
    ParentDirectory,
    /// The name contained a control character or a NUL.
    ControlCharacter,
    /// The name is reserved by Windows.
    ReservedWindowsName(String),
    /// The name ended in a dot or a space, which Windows silently drops.
    TrailingDotOrSpace,
    /// The name was longer than the accepted budget.
    NameTooLong,
    /// A single component was too long.
    ComponentTooLong,
    /// The payload exceeded the configured byte budget.
    TooManyBytes {
        limit: usize,
        found: usize,
    },
    /// The image dimensions were outside the configured budget.
    ImageDimensions {
        width: u32,
        height: u32,
    },
    /// The image header did not agree with the declared dimensions.
    DimensionMismatch {
        declared: (u32, u32),
        header: (u32, u32),
    },
    /// The bytes were not a recognizable image of an accepted type.
    UnrecognizedImage,
    /// The declared media type is not accepted.
    UnsupportedMediaType(String),
}

/// Avatar decode budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AvatarLimits {
    pub max_bytes: usize,
    pub max_dimension: u32,
    pub min_dimension: u32,
    pub max_pixels: u64,
}

impl Default for AvatarLimits {
    fn default() -> Self {
        Self {
            max_bytes: MAX_AVATAR_BYTES,
            max_dimension: MAX_AVATAR_DIMENSION,
            min_dimension: MIN_AVATAR_DIMENSION,
            max_pixels: MAX_AVATAR_PIXELS,
        }
    }
}

/// A validated avatar header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AvatarInfo {
    pub width: u32,
    pub height: u32,
    pub bytes: usize,
    pub media_type: &'static str,
}

/// Reads the image dimensions out of the supported headers without decoding pixels.
#[must_use]
pub fn read_image_header(bytes: &[u8]) -> Option<(&'static str, u32, u32)> {
    if let Some((width, height)) = read_png(bytes) {
        return Some(("image/png", width, height));
    }
    if let Some((width, height)) = read_gif(bytes) {
        return Some(("image/gif", width, height));
    }
    if let Some((width, height)) = read_webp(bytes) {
        return Some(("image/webp", width, height));
    }
    if let Some((width, height)) = read_jpeg(bytes) {
        return Some(("image/jpeg", width, height));
    }
    None
}

/// Pure avatar validator: byte budget, dimension budget, and header agreement.
///
/// `declared_width`/`declared_height` are the values the peer put in
/// [`crate::payloads::AvatarDataPayload`]; a mismatch is refused rather than trusted.
pub fn validate_avatar_image(
    bytes: &[u8],
    declared_width: u32,
    declared_height: u32,
    declared_media_type: &str,
    limits: &AvatarLimits,
) -> Result<AvatarInfo, SecurityError> {
    if bytes.is_empty() {
        return Err(SecurityError::Empty);
    }
    if bytes.len() > limits.max_bytes {
        return Err(SecurityError::TooManyBytes {
            limit: limits.max_bytes,
            found: bytes.len(),
        });
    }
    let (format, width, height) = read_image_header(bytes).ok_or(SecurityError::UnrecognizedImage)?;
    if !ACCEPTED_AVATAR_MEDIA_TYPES.contains(&format) {
        return Err(SecurityError::UnsupportedMediaType(format.to_owned()));
    }
    if !declared_media_type.is_empty() && declared_media_type != format {
        return Err(SecurityError::UnsupportedMediaType(
            declared_media_type.to_owned(),
        ));
    }
    if declared_width != 0
        && declared_height != 0
        && (declared_width, declared_height) != (width, height)
    {
        return Err(SecurityError::DimensionMismatch {
            declared: (declared_width, declared_height),
            header: (width, height),
        });
    }
    if width < limits.min_dimension
        || height < limits.min_dimension
        || width > limits.max_dimension
        || height > limits.max_dimension
    {
        return Err(SecurityError::ImageDimensions { width, height });
    }
    let pixels = u64::from(width).saturating_mul(u64::from(height));
    if pixels > limits.max_pixels {
        return Err(SecurityError::ImageDimensions { width, height });
    }
    Ok(AvatarInfo {
        width,
        height,
        bytes: bytes.len(),
        media_type: format,
    })
}

fn be32(bytes: &[u8], offset: usize) -> Option<u32> {
    let window = <[u8; 4]>::try_from(bytes.get(offset..offset + 4)?).ok()?;
    Some(u32::from_be_bytes(window))
}

fn le32(bytes: &[u8], offset: usize) -> Option<u32> {
    let window = <[u8; 4]>::try_from(bytes.get(offset..offset + 4)?).ok()?;
    Some(u32::from_le_bytes(window))
}

fn le16(bytes: &[u8], offset: usize) -> Option<u32> {
    let window = <[u8; 2]>::try_from(bytes.get(offset..offset + 2)?).ok()?;
    Some(u32::from_le_bytes([window[0], window[1], 0, 0]))
}

fn le24(bytes: &[u8], offset: usize) -> Option<u32> {
    let window = <[u8; 3]>::try_from(bytes.get(offset..offset + 3)?).ok()?;
    Some(u32::from_le_bytes([window[0], window[1], window[2], 0]))
}

fn read_png(bytes: &[u8]) -> Option<(u32, u32)> {
    const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    if bytes.len() < 24 || bytes[..8] != SIGNATURE {
        return None;
    }
    // IHDR must be the first chunk: length, "IHDR", 4-byte width, 4-byte height.
    if be32(bytes, 8)? != 13 || bytes.get(12..16) != Some(b"IHDR") {
        return None;
    }
    let width = be32(bytes, 16)?;
    let height = be32(bytes, 20)?;
    if width == 0 || height == 0 {
        return None;
    }
    Some((width, height))
}

fn read_gif(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 10 || !bytes[..6].starts_with(b"GIF8") {
        return None;
    }
    let width = le16(bytes, 6)?;
    let height = le16(bytes, 8)?;
    if width == 0 || height == 0 {
        return None;
    }
    Some((width, height))
}

/// Walks JPEG markers to the first start-of-frame segment.
fn read_jpeg(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 4 || bytes[0] != 0xff || bytes[1] != 0xd8 {
        return None;
    }
    let mut offset = 2_usize;
    while offset + 10 <= bytes.len() {
        if bytes[offset] != 0xff {
            offset += 1;
            continue;
        }
        let marker = bytes[offset + 1];
        if marker == 0xff {
            // Fill byte.
            offset += 1;
            continue;
        }
        // Standalone markers carry no length field.
        if marker == 0x01 || matches!(marker, 0xd0..=0xd9) {
            offset += 2;
            continue;
        }
        let length = u16::from_be_bytes([bytes[offset + 2], bytes[offset + 3]]) as usize;
        if length < 2 {
            return None;
        }
        // SOF0..SOF3 and SOF5..SOF7, SOF9..SOF11, SOF13..SOF15 carry the frame header.
        if matches!(marker, 0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf) {
            let height = u16::from_be_bytes([bytes[offset + 5], bytes[offset + 6]]) as u32;
            let width = u16::from_be_bytes([bytes[offset + 7], bytes[offset + 8]]) as u32;
            return if width == 0 || height == 0 {
                None
            } else {
                Some((width, height))
            };
        }
        offset += 2 + length;
    }
    None
}

fn read_webp(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 16 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WEBP" {
        return None;
    }
    let mut cursor = 12_usize;
    while cursor + 8 <= bytes.len() {
        let fourcc = &bytes[cursor..cursor + 4];
        let data = cursor + 8;
        if matches!(fourcc, b"VP8X" | b"VP8 " | b"VP8L") {
            return read_webp_frame(bytes, data, fourcc);
        }
        let size = le32(bytes, cursor + 4)? as usize;
        cursor = data.checked_add(size)?.checked_add(size % 2)?;
    }
    None
}

fn read_webp_frame(bytes: &[u8], data: usize, fourcc: &[u8]) -> Option<(u32, u32)> {
    match fourcc {
        b"VP8X" => {
            // flags(1) + reserved(3) + canvas width-1 (3 LE) + canvas height-1 (3 LE).
            let width = le24(bytes, data + 4)? + 1;
            let height = le24(bytes, data + 7)? + 1;
            Some((width, height))
        }
        b"VP8 " => {
            // 3-byte frame tag, then a 3-byte start code and two 14-bit sizes.
            if bytes.get(data + 3..data + 6)? != [0x9d, 0x01, 0x2a] {
                return None;
            }
            let width = le16(bytes, data + 6)? & 0x3f_ff;
            let height = le16(bytes, data + 8)? & 0x3f_ff;
            if width == 0 || height == 0 {
                return None;
            }
            Some((width, height))
        }
        b"VP8L" => {
            if *bytes.get(data)? != 0x2f {
                return None;
            }
            let bits = le32(bytes, data + 1)?;
            let width = (bits & 0x3f_ff) + 1;
            let height = ((bits >> 14) & 0x3f_ff) + 1;
            Some((width, height))
        }
        _ => None,
    }
}

/// True for the Windows device names that would hijack a received file.
#[must_use]
pub fn is_reserved_windows_name(name: &str) -> Option<String> {
    let stem = name.split(['.', ' ']).next().unwrap_or(name);
    let stem = stem.to_ascii_uppercase();
    let reserved = matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$" | "CONIN$" | "CONOUT$"
    ) || ((stem.starts_with("COM") || stem.starts_with("LPT"))
        && stem.len() == 4
        && stem[3..].bytes().all(|byte| byte.is_ascii_digit())
        && !stem[3..].starts_with('0'));
    reserved.then_some(stem)
}

/// Validates a path-like file name a peer sent us.
///
/// Rejects absolute paths, `..` traversal, control characters, empty or overlong names,
/// trailing dots and spaces, and Windows reserved device names. Nothing is created,
/// canonicalized or touched: this is a pure string check.
pub fn validate_received_file_name(name: &str) -> Result<(), SecurityError> {
    if name.is_empty() {
        return Err(SecurityError::Empty);
    }
    if name.len() > MAX_PATH_BYTES {
        return Err(SecurityError::NameTooLong);
    }
    if name.chars().any(char::is_control) {
        return Err(SecurityError::ControlCharacter);
    }
    let bytes = name.as_bytes();
    // Absolute POSIX paths, drive letters, and UNC shares.
    if bytes.starts_with(b"/") || bytes.starts_with(b"\\") {
        return Err(SecurityError::AbsolutePath);
    }
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return Err(SecurityError::AbsolutePath);
    }
    if name.ends_with('.') || name.ends_with(' ') {
        return Err(SecurityError::TrailingDotOrSpace);
    }
    for component in name.split(['/', '\\']) {
        if component == ".." {
            return Err(SecurityError::ParentDirectory);
        }
        if component.len() > MAX_FILE_NAME_BYTES {
            return Err(SecurityError::ComponentTooLong);
        }
        if let Some(reserved) = is_reserved_windows_name(component) {
            return Err(SecurityError::ReservedWindowsName(reserved));
        }
    }
    Ok(())
}

/// True when the name is a single safe path component, the stricter rule used by
/// [`crate::files::FileOffer::validate`].
#[must_use]
pub fn is_safe_single_component(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\\')
        && validate_received_file_name(name).is_ok()
}

/// Token bucket rate limit configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RateLimit {
    /// Sustained events per second.
    pub refill_per_s: u64,
    /// Burst allowance.
    pub burst: u64,
}

impl Default for RateLimit {
    fn default() -> Self {
        Self {
            refill_per_s: 50,
            burst: 100,
        }
    }
}

/// A single peer's bucket, advanced by explicit timestamps: no clocks are read here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TokenBucket {
    limit: RateLimit,
    tokens: u64,
    updated_ms: u64,
}

impl TokenBucket {
    #[must_use]
    pub fn new(limit: RateLimit, now_ms: u64) -> Self {
        Self {
            limit,
            tokens: limit.burst,
            updated_ms: now_ms,
        }
    }

    /// Refills and spends `cost`; false when the peer is over budget.
    pub fn allow(&mut self, now_ms: u64, cost: u64) -> bool {
        self.tokens = self.refilled(now_ms);
        self.updated_ms = self.updated_ms.max(now_ms);
        if cost == 0 {
            return true;
        }
        if self.tokens >= cost {
            self.tokens -= cost;
            true
        } else {
            false
        }
    }

    /// Milliseconds until `cost` tokens are available, when it is not available now.
    #[must_use]
    pub fn retry_after_ms(&self, now_ms: u64, cost: u64) -> Option<u64> {
        let available = self.refilled(now_ms);
        if available >= cost {
            return None;
        }
        if self.limit.refill_per_s == 0 {
            return Some(u64::MAX);
        }
        Some((cost - available).saturating_mul(1_000).div_ceil(self.limit.refill_per_s))
    }

    /// Available tokens at `now_ms`, without spending or mutating anything.
    #[must_use]
    pub fn available(&self, now_ms: u64) -> u64 {
        self.refilled(now_ms)
    }

    fn refilled(&self, now_ms: u64) -> u64 {
        if now_ms <= self.updated_ms {
            return self.tokens;
        }
        let elapsed = now_ms - self.updated_ms;
        let gained = elapsed.saturating_mul(self.limit.refill_per_s) / 1_000;
        self.tokens.saturating_add(gained).min(self.limit.burst)
    }
}

/// Per-peer token buckets with a bounded peer set, so a flood of spoofed ids cannot grow
/// the table without limit.
#[derive(Clone, Debug)]
pub struct PeerRateLimiter {
    limit: RateLimit,
    buckets: HashMap<String, TokenBucket>,
    max_peers: usize,
    idle_ms: u64,
}

impl Default for PeerRateLimiter {
    fn default() -> Self {
        Self::new(RateLimit::default())
    }
}

impl PeerRateLimiter {
    #[must_use]
    pub fn new(limit: RateLimit) -> Self {
        Self::with_capacity(limit, 1_024, 10 * 60 * 1_000)
    }

    #[must_use]
    pub fn with_capacity(limit: RateLimit, max_peers: usize, idle_ms: u64) -> Self {
        Self {
            limit,
            buckets: HashMap::new(),
            max_peers: max_peers.max(1),
            idle_ms,
        }
    }

    /// Spend `cost` from the peer's bucket; false blocks the event.
    ///
    /// An unknown peer is admitted only while the table has room, so a peer-id flood
    /// cannot exhaust memory.
    pub fn allow(&mut self, peer_id: &str, now_ms: u64, cost: u64) -> bool {
        if let Some(bucket) = self.buckets.get_mut(peer_id) {
            return bucket.allow(now_ms, cost);
        }
        if self.buckets.len() >= self.max_peers {
            self.evict_expired(now_ms);
            if self.buckets.len() >= self.max_peers {
                return false;
            }
        }
        let mut bucket = TokenBucket::new(self.limit, now_ms);
        let allowed = bucket.allow(now_ms, cost);
        self.buckets.insert(peer_id.to_owned(), bucket);
        allowed
    }

    /// Remaining budget for a peer, or the full burst when unseen.
    #[must_use]
    pub fn available(&self, peer_id: &str, now_ms: u64) -> u64 {
        self.buckets
            .get(peer_id)
            .map(|bucket| bucket.available(now_ms))
            .unwrap_or(self.limit.burst)
    }

    #[must_use]
    pub fn retry_after_ms(&self, peer_id: &str, now_ms: u64, cost: u64) -> Option<u64> {
        self.buckets
            .get(peer_id)
            .and_then(|bucket| bucket.retry_after_ms(now_ms, cost))
    }

    /// Drops peers idle for longer than `idle_ms`; returns how many were removed.
    pub fn evict_expired(&mut self, now_ms: u64) -> usize {
        let before = self.buckets.len();
        self.buckets
            .retain(|_, bucket| now_ms.saturating_sub(bucket.updated_ms) < self.idle_ms);
        before - self.buckets.len()
    }

    #[must_use]
    pub fn tracked_peers(&self) -> usize {
        self.buckets.len()
    }

    #[must_use]
    pub fn limit(&self) -> RateLimit {
        self.limit
    }
}

/// Rejects a claimed frame length before any allocation happens.
#[must_use]
pub fn frame_length_is_bounded(length: usize, max_bytes: usize) -> bool {
    length <= max_bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::MAX_CONTROL_FRAME_BYTES;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        bytes.extend_from_slice(&13_u32.to_be_bytes());
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&[8, 6, 0, 0, 0]);
        bytes
    }

    fn gif(width: u16, height: u16) -> Vec<u8> {
        let mut bytes = b"GIF89a".to_vec();
        bytes.extend_from_slice(&width.to_le_bytes());
        bytes.extend_from_slice(&height.to_le_bytes());
        bytes.push(0);
        bytes
    }

    fn webp_lossy(width: u16, height: u16) -> Vec<u8> {
        let mut bytes = b"RIFF".to_vec();
        bytes.extend_from_slice(&64_u32.to_le_bytes());
        bytes.extend_from_slice(b"WEBP");
        bytes.extend_from_slice(b"VP8 ");
        bytes.extend_from_slice(&13_u32.to_le_bytes());
        bytes.extend_from_slice(&[0x00, 0x00, 0x00]); // frame tag
        bytes.extend_from_slice(&[0x9d, 0x01, 0x2a]); // start code
        bytes.extend_from_slice(&width.to_le_bytes());
        bytes.extend_from_slice(&height.to_le_bytes());
        bytes
    }

    fn webp_extended(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = b"RIFF".to_vec();
        bytes.extend_from_slice(&64_u32.to_le_bytes());
        bytes.extend_from_slice(b"WEBP");
        bytes.extend_from_slice(b"VP8X");
        bytes.extend_from_slice(&10_u32.to_le_bytes());
        bytes.extend_from_slice(&[0x10, 0, 0, 0]); // flags + reserved
        bytes.extend_from_slice(&[
            (width - 1) as u8,
            ((width - 1) >> 8) as u8,
            ((width - 1) >> 16) as u8,
            (height - 1) as u8,
            ((height - 1) >> 8) as u8,
            ((height - 1) >> 16) as u8,
        ]);
        bytes
    }

    fn jpeg(width: u16, height: u16) -> Vec<u8> {
        let mut bytes = vec![0xff, 0xd8];
        bytes.extend_from_slice(&[0xff, 0xe0]);
        bytes.extend_from_slice(&16_u16.to_be_bytes());
        bytes.extend_from_slice(b"JFIF\0");
        bytes.extend_from_slice(&[1, 1, 0, 0, 1, 0, 1, 0, 0]);
        bytes.extend_from_slice(&[0xff, 0xc0]);
        bytes.extend_from_slice(&17_u16.to_be_bytes());
        bytes.push(8);
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.push(3);
        bytes
    }

    #[test]
    fn avatar_headers_are_read_without_decoding() {
        assert_eq!(read_image_header(&png(64, 64)), Some(("image/png", 64, 64)));
        assert_eq!(
            read_image_header(&gif(32, 16)),
            Some(("image/gif", 32, 16))
        );
        assert_eq!(
            read_image_header(&webp_lossy(20, 40)),
            Some(("image/webp", 20, 40))
        );
        assert_eq!(
            read_image_header(&webp_extended(101, 202)),
            Some(("image/webp", 101, 202))
        );
        assert_eq!(
            read_image_header(&jpeg(80, 60)),
            Some(("image/jpeg", 80, 60))
        );
        assert_eq!(read_image_header(b"not an image"), None);
        assert_eq!(read_image_header(&[]), None);
        // Truncated headers are refused rather than guessed.
        let mut truncated = png(64, 64);
        truncated.truncate(18);
        assert_eq!(read_image_header(&truncated), None);
    }

    #[test]
    fn avatar_byte_and_dimension_limits_are_enforced() {
        let limits = AvatarLimits::default();
        let header = png(256, 256);
        let info =
            validate_avatar_image(&header, 256, 256, "image/png", &limits).expect("valid avatar");
        assert_eq!(
            info,
            AvatarInfo {
                width: 256,
                height: 256,
                bytes: header.len(),
                media_type: "image/png"
            }
        );
        assert_eq!(
            validate_avatar_image(&png(4_096, 4_096), 4_096, 4_096, "image/png", &limits),
            Err(SecurityError::ImageDimensions {
                width: 4_096,
                height: 4_096
            })
        );
        assert_eq!(
            validate_avatar_image(&png(0, 0), 0, 0, "image/png", &limits),
            Err(SecurityError::UnrecognizedImage)
        );
        let tiny = AvatarLimits {
            max_bytes: 8,
            ..limits
        };
        assert_eq!(
            validate_avatar_image(&png(16, 16), 16, 16, "image/png", &tiny),
            Err(SecurityError::TooManyBytes {
                limit: 8,
                found: png(16, 16).len()
            })
        );
        assert_eq!(
            validate_avatar_image(&[], 1, 1, "image/png", &limits),
            Err(SecurityError::Empty)
        );
        // The pixel budget catches narrow shapes that pass the edge cap.
        let narrow = AvatarLimits {
            max_pixels: 1_024,
            ..limits
        };
        assert_eq!(
            validate_avatar_image(&png(1_024, 1_024), 1_024, 1_024, "image/png", &narrow),
            Err(SecurityError::ImageDimensions {
                width: 1_024,
                height: 1_024
            })
        );
        let strict_min = AvatarLimits {
            min_dimension: 16,
            ..limits
        };
        assert_eq!(
            validate_avatar_image(&png(8, 8), 8, 8, "image/png", &strict_min),
            Err(SecurityError::ImageDimensions {
                width: 8,
                height: 8
            })
        );
    }

    #[test]
    fn avatar_declared_metadata_must_match_the_bytes() {
        let limits = AvatarLimits::default();
        assert_eq!(
            validate_avatar_image(&png(64, 64), 512, 512, "image/png", &limits),
            Err(SecurityError::DimensionMismatch {
                declared: (512, 512),
                header: (64, 64)
            })
        );
        assert_eq!(
            validate_avatar_image(&png(64, 64), 64, 64, "image/heif", &limits),
            Err(SecurityError::UnsupportedMediaType("image/heif".into()))
        );
        assert_eq!(
            validate_avatar_image(&gif(64, 64), 64, 64, "image/png", &limits),
            Err(SecurityError::UnsupportedMediaType("image/png".into()))
        );
        // An undeclared media type still has to be an accepted format; declared zero
        // dimensions fall back to the header.
        assert!(validate_avatar_image(&png(64, 64), 0, 0, "", &limits).is_ok());
    }

    #[test]
    fn received_file_names_reject_traversal_and_device_names() {
        assert!(validate_received_file_name("report.pdf").is_ok());
        assert!(validate_received_file_name("notes/2026/plan.md").is_ok());
        for hostile in [
            "/etc/passwd",
            "\\Windows\\system32\\cmd.exe",
            "C:\\Windows\\system32\\cmd.exe",
            "../../etc/passwd",
            "ok/../../etc/passwd",
            "..",
            "a\x00.txt",
            "evil\x01.txt",
            "line\x0break.txt",
            "nul",
            "NUL.txt",
            "con.txt",
            "COM1.log",
            "com9",
            "LPT1",
            "aux.png",
            "trailing.",
            "trailing ",
        ] {
            assert!(
                validate_received_file_name(hostile).is_err(),
                "{hostile:?} must be refused"
            );
        }
        assert_eq!(validate_received_file_name(""), Err(SecurityError::Empty));
        assert_eq!(
            validate_received_file_name("/abs"),
            Err(SecurityError::AbsolutePath)
        );
        assert_eq!(
            validate_received_file_name("../x"),
            Err(SecurityError::ParentDirectory)
        );
        assert_eq!(
            validate_received_file_name("nul"),
            Err(SecurityError::ReservedWindowsName("NUL".into()))
        );
        assert_eq!(
            validate_received_file_name(&"a".repeat(MAX_PATH_BYTES + 1)),
            Err(SecurityError::NameTooLong)
        );
        assert_eq!(
            validate_received_file_name(&format!("{}.txt", "a".repeat(MAX_FILE_NAME_BYTES + 1))),
            Err(SecurityError::ComponentTooLong)
        );
        // Look-alikes are allowed.
        assert!(validate_received_file_name("console.txt").is_ok());
        assert!(validate_received_file_name("COM10.log").is_ok());
        assert!(validate_received_file_name("nulles.png").is_ok());
        assert!(validate_received_file_name("report_2026-01-01.pdf").is_ok());
    }

    #[test]
    fn single_component_rule_matches_the_file_offer_validator() {
        assert!(is_safe_single_component("photo.jpg"));
        assert!(!is_safe_single_component("dir/photo.jpg"));
        assert!(!is_safe_single_component(".."));
        assert!(!is_safe_single_component(""));
        assert!(!is_safe_single_component("nul"));
        for candidate in ["photo.jpg", "dir/photo.jpg", "..", "", "nul", "a\\b.txt"] {
            let offer = crate::files::FileOffer {
                id: uuid::Uuid::now_v7(),
                file_name: candidate.into(),
                size: 1,
                blake3_hash: [0; 32],
            };
            let offer_ok = offer
                .validate(crate::files::DEFAULT_MAX_FILE_BYTES)
                .is_ok();
            assert_eq!(
                offer_ok,
                is_safe_single_component(candidate),
                "{candidate:?} must be judged the same way by both rules"
            );
        }
    }

    #[test]
    fn token_bucket_honours_burst_then_refill() {
        let limit = RateLimit {
            refill_per_s: 10,
            burst: 5,
        };
        let mut bucket = TokenBucket::new(limit, 0);
        for _ in 0..5 {
            assert!(bucket.allow(0, 1));
        }
        assert!(!bucket.allow(0, 1));
        // 100 ms buys exactly one token at 10/s.
        assert!(bucket.allow(100, 1));
        assert!(!bucket.allow(150, 1));
        assert_eq!(bucket.retry_after_ms(150, 1), Some(100));
        assert_eq!(bucket.retry_after_ms(250, 1), None);
        // Refill is capped at the burst, never banked forever.
        assert_eq!(bucket.available(10_000), 5);
        assert!(bucket.allow(10_000, 5));
        assert_eq!(bucket.available(10_000), 0);
        assert!(bucket.allow(10_000, 0));
        assert_eq!(
            TokenBucket::new(RateLimit { refill_per_s: 0, burst: 1 }, 0).retry_after_ms(0, 2),
            Some(u64::MAX)
        );
    }

    #[test]
    fn buckets_never_go_backwards_with_skewed_clocks() {
        let mut bucket = TokenBucket::new(
            RateLimit {
                refill_per_s: 10,
                burst: 3,
            },
            1_000,
        );
        assert!(bucket.allow(1_000, 1));
        assert!(bucket.allow(500, 1));
        assert_eq!(bucket.available(100), 1);
    }

    #[test]
    fn per_peer_limiter_isolates_peers_and_bounds_the_table() {
        let limit = RateLimit {
            refill_per_s: 1,
            burst: 2,
        };
        let mut limiter = PeerRateLimiter::with_capacity(limit, 2, 1_000);
        assert!(limiter.allow("peer-a", 0, 1));
        assert!(limiter.allow("peer-a", 0, 1));
        assert!(!limiter.allow("peer-a", 0, 1), "peer-a is spent");
        assert!(limiter.allow("peer-b", 0, 1), "peer-b has its own budget");
        assert_eq!(limiter.tracked_peers(), 2);
        assert_eq!(limiter.retry_after_ms("peer-a", 0, 1), Some(1_000));
        assert_eq!(limiter.available("peer-a", 1_000), 1);
        // A flood of unknown ids cannot grow the table past the cap.
        assert!(!limiter.allow("peer-c", 0, 1));
        assert_eq!(limiter.tracked_peers(), 2);
        // Idle peers are reclaimed.
        assert_eq!(limiter.evict_expired(60_000), 2);
        assert_eq!(limiter.tracked_peers(), 0);
        assert!(limiter.allow("peer-c", 60_000, 1));
        assert_eq!(limiter.limit(), limit);
        assert_eq!(limiter.available("peer-never-seen", 0), 2);
        assert_eq!(limiter.retry_after_ms("peer-never-seen", 0, 1), None);
    }

    #[test]
    fn frame_length_guard() {
        assert!(frame_length_is_bounded(
            MAX_CONTROL_FRAME_BYTES,
            MAX_CONTROL_FRAME_BYTES
        ));
        assert!(!frame_length_is_bounded(
            MAX_CONTROL_FRAME_BYTES + 1,
            MAX_CONTROL_FRAME_BYTES
        ));
    }
}
