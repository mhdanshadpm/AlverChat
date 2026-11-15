//! Typed payload models for every protocol envelope type.
//!
//! The wire format stays a JSON envelope (see [`crate::protocol`]) whose payload is
//! structurally untyped. This layer gives the desktop and UI code an exhaustive,
//! versionable model over it:
//!
//! * every envelope type declared by the protocol has a typed payload struct;
//! * [`classify`] converts a received envelope into a [`Payload`];
//! * [`dispatch`] maps an envelope onto the engine subsystem that owns it;
//! * [`decode_payload_frame`] decodes bytes without ever erroring on an unknown type;
//! * an unknown type, a malformed payload, or an out-of-bounds field decodes to
//!   [`Payload::Ignored`] with a reason. Nothing here rejects data a peer chose to send
//!   us with a hard error, so a newer peer can never wedge an older receiver.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::groups::GroupState;
use crate::protocol::{ProtocolError, MAX_CONTROL_FRAME_BYTES};
use crate::{Envelope, Hlc, PROTOCOL_VERSION};

/// Largest accepted chat body, mirroring the guard in [`crate::storage::Store`].
pub const MAX_BODY_BYTES: usize = 16_384;
/// Largest accepted reaction sequence.
pub const MAX_REACTION_BYTES: usize = 32;
/// Largest accepted avatar payload, matching the decoder limits in [`crate::security`].
pub const MAX_AVATAR_BYTES: usize = 512 * 1024;
/// Largest accepted session description payload.
pub const MAX_SDP_BYTES: usize = 256 * 1024;
/// Largest accepted ICE candidate string.
pub const MAX_CANDIDATE_BYTES: usize = 2048;
/// Largest number of attachments declared by a single message.
pub const MAX_ATTACHMENTS: usize = 64;

/// Envelope type names. These are the single source of truth for
/// [`crate::protocol`]'s known-type table.
pub mod kind {
    /// The LAN passphrase challenge this build exchanges right after the session opens:
    /// a random nonce plus a keyed BLAKE3 MAC over both peer ids, or `mac: null` from a
    /// device that has no passphrase. Both sides must agree, or neither talks.
    pub const AUTH: &str = "auth";
    pub const HELLO: &str = "hello";
    pub const HELLO_ACK: &str = "hello_ack";
    pub const PRESENCE: &str = "presence";
    pub const PROFILE_UPDATE: &str = "profile_update";
    pub const AVATAR_REQUEST: &str = "avatar_request";
    pub const AVATAR_DATA: &str = "avatar_data";
    pub const MESSAGE: &str = "message";
    pub const MESSAGE_EDIT: &str = "message_edit";
    pub const MESSAGE_DELETE: &str = "message_delete";
    pub const REACTION: &str = "reaction";
    pub const RECEIPT: &str = "receipt";
    pub const TYPING: &str = "typing";
    pub const PIN_UPDATE: &str = "pin_update";
    /// One signed `GroupState` successor: the frame this build puts on the wire for a
    /// create, a rename, an icon change or a membership change, because the state object
    /// carries its own epoch and signature and needs no per-change envelope. Its payload
    /// is the state itself; the four `group_*` kinds below are the older split design that
    /// wraps it under `state`, and nothing in this build sends one.
    pub const GROUP_STATE: &str = "group";
    pub const GROUP_CREATE: &str = "group_create";
    pub const GROUP_UPDATE: &str = "group_update";
    pub const GROUP_MEMBER_CHANGE: &str = "group_member_change";
    pub const GROUP_ICON_UPDATE: &str = "group_icon_update";
    pub const ANNOUNCEMENT: &str = "announcement";
    pub const ANNOUNCEMENT_ACK: &str = "announcement_ack";
    pub const FILE_OFFER: &str = "file_offer";
    pub const FILE_ACCEPT: &str = "file_accept";
    pub const FILE_CHUNK: &str = "file_chunk";
    pub const FILE_ACK: &str = "file_ack";
    pub const FILE_CANCEL: &str = "file_cancel";
    pub const CALL_OFFER: &str = "call_offer";
    pub const CALL_ANSWER: &str = "call_answer";
    pub const ICE_CANDIDATE: &str = "ice_candidate";
    pub const CALL_END: &str = "call_end";
    pub const CALL_REJECT: &str = "call_reject";
    pub const CALL_BUSY: &str = "call_busy";
    pub const SYNC_REQUEST: &str = "sync_request";
    pub const PING: &str = "ping";
    pub const PONG: &str = "pong";
    pub const BYE: &str = "bye";
}

/// Every envelope type the protocol knows about, in declaration order.
pub const ALL_TYPES: &[&str] = &[
    kind::AUTH,
    kind::HELLO,
    kind::HELLO_ACK,
    kind::PRESENCE,
    kind::PROFILE_UPDATE,
    kind::AVATAR_REQUEST,
    kind::AVATAR_DATA,
    kind::MESSAGE,
    kind::MESSAGE_EDIT,
    kind::MESSAGE_DELETE,
    kind::REACTION,
    kind::RECEIPT,
    kind::TYPING,
    kind::PIN_UPDATE,
    kind::GROUP_STATE,
    kind::GROUP_CREATE,
    kind::GROUP_UPDATE,
    kind::GROUP_MEMBER_CHANGE,
    kind::GROUP_ICON_UPDATE,
    kind::ANNOUNCEMENT,
    kind::ANNOUNCEMENT_ACK,
    kind::FILE_OFFER,
    kind::FILE_ACCEPT,
    kind::FILE_CHUNK,
    kind::FILE_ACK,
    kind::FILE_CANCEL,
    kind::CALL_OFFER,
    kind::CALL_ANSWER,
    kind::ICE_CANDIDATE,
    kind::CALL_END,
    kind::CALL_REJECT,
    kind::CALL_BUSY,
    kind::SYNC_REQUEST,
    kind::PING,
    kind::PONG,
    kind::BYE,
];

/// True when a type name is declared by this build.
#[must_use]
pub fn is_known_type(kind: &str) -> bool {
    ALL_TYPES.contains(&kind)
}

/// A file attachment reference carried inside a message payload.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct AttachmentRef {
    pub transfer_id: Uuid,
    pub file_name: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub blake3_hash: Vec<u8>,
}

/// One attachment inside a message.
pub type MessageAttachment = AttachmentRef;

/// A typed reference to a stored message, used by the reaction and receipt layers.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessageRef {
    pub chat_id: String,
    pub message_id: Uuid,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuthPayload {
    /// Base64 of 32 fresh random bytes, chosen by whoever sends it first.
    pub nonce: String,
    /// Base64 keyed MAC over the secret, both peer ids and this frame's nonce. Absent on a
    /// device that set no passphrase, which is what makes a half-set pair fail on both ends.
    #[serde(default)]
    pub mac: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct HelloPayload {
    pub peer_id: String,
    pub display_name: String,
    #[serde(default)]
    pub supported_types: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct HelloAckPayload {
    pub peer_id: String,
    #[serde(default = "default_true")]
    pub accepted: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PresencePayload {
    pub peer_id: String,
    /// One of `online`, `away`, `busy`, `invisible`.
    pub status: String,
    #[serde(default)]
    pub since_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProfileUpdatePayload {
    pub peer_id: String,
    pub display_name: String,
    #[serde(default)]
    pub status_text: String,
    /// Hex hash of the avatar the peer currently publishes, when it has one.
    #[serde(default)]
    pub avatar_hash: Option<String>,
    #[serde(default)]
    pub updated_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AvatarRequestPayload {
    pub peer_id: String,
    /// Hash the requester already holds; an empty response means "unchanged".
    #[serde(default)]
    pub have_hash: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AvatarDataPayload {
    pub peer_id: String,
    /// Hex blake3 hash of `bytes`.
    pub hash: String,
    #[serde(default)]
    pub media_type: String,
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
    #[serde(default, with = "bytes_codec")]
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessagePayload {
    pub chat_id: String,
    /// The body, under the key the desktop both writes and requires: a frame without `text`
    /// is refused by the reader, so the table names it the same way.
    pub text: String,
    #[serde(default)]
    pub reply_to: Option<Uuid>,
    /// A copy of something forwarded from elsewhere. A frame from an older build carries no
    /// key, and the honest reading of that is an original.
    #[serde(default)]
    pub forwarded: bool,
    #[serde(default)]
    pub attachments: Vec<MessageAttachment>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessageEditPayload {
    pub message_id: Uuid,
    pub body: String,
    /// Replacement clock of the edited body.
    pub clock: Hlc,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessageDeletePayload {
    pub message_id: Uuid,
    /// `false` requests a delete-for-me; the durable tombstone path is `true`.
    #[serde(default = "default_true")]
    pub for_everyone: bool,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReactionPayload {
    pub message: MessageRef,
    pub emoji: String,
    #[serde(default)]
    pub remove: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReceiptPayload {
    pub message: MessageRef,
    /// `delivered` or `read`.
    pub kind: String,
    #[serde(default)]
    pub occurred_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TypingPayload {
    pub chat_id: String,
    #[serde(default)]
    pub typing: bool,
    /// Optional typing-group token.
    #[serde(default)]
    pub token: Option<Uuid>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PinUpdatePayload {
    pub chat_id: String,
    pub message_id: Uuid,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub position: u32,
}

/// The payload of [`kind::GROUP_STATE`]: a signed `GroupState`, sent bare.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct GroupStatePayload(pub GroupState);

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct GroupCreatePayload {
    pub state: GroupState,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct GroupUpdatePayload {
    pub state: GroupState,
    #[serde(default)]
    pub note: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct GroupMemberChangePayload {
    pub state: GroupState,
    /// Peer the change is about.
    pub member: String,
    /// `join`, `leave`, `removed`, `muted` or `unmuted`.
    pub change: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct GroupIconUpdatePayload {
    pub group_id: String,
    #[serde(default)]
    pub icon_color: String,
    #[serde(default)]
    pub emoji: String,
    pub epoch: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AnnouncementPayload {
    pub announcement_id: Uuid,
    pub group_id: String,
    /// The body, under the key the desktop both writes and requires: `text`, not `body`.
    /// A round-trip fixture cannot tell the two apart, so the name has to match the wire.
    pub text: String,
    #[serde(default)]
    pub created_ms: u64,
    #[serde(default)]
    pub admins_only: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AnnouncementAckPayload {
    pub announcement_id: Uuid,
    pub peer_id: String,
    #[serde(default)]
    pub acked_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileOfferPayload {
    pub transfer_id: Uuid,
    pub chat_id: String,
    pub file_name: String,
    pub size: u64,
    pub blake3_hash: Vec<u8>,
    #[serde(default)]
    pub note: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileAcceptPayload {
    pub transfer_id: Uuid,
    #[serde(default)]
    pub resume_from: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileChunkPayload {
    pub transfer_id: Uuid,
    pub index: u64,
    #[serde(with = "bytes_codec")]
    pub bytes: Vec<u8>,
    #[serde(default)]
    pub last: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileAckPayload {
    pub transfer_id: Uuid,
    /// Highest contiguous chunk index the receiver durably holds.
    pub contiguous_index: u64,
    #[serde(default)]
    pub complete: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileCancelPayload {
    pub transfer_id: Uuid,
    #[serde(default)]
    pub reason: Option<String>,
}

/// Base64 encoding for raw chunk bytes.
///
/// A JSON number array costs up to four characters per byte, so a 40 KiB chunk would
/// no longer fit one transport frame; base64 keeps it at 1.33.
mod bytes_codec {
    use base64::Engine as _;
    use serde::{de::Error as _, Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&base64::engine::general_purpose::STANDARD.encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        let text = String::deserialize(deserializer)?;
        base64::engine::general_purpose::STANDARD
            .decode(text.as_bytes())
            .map_err(D::Error::custom)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CallOfferPayload {
    pub call_id: Uuid,
    pub chat_id: String,
    /// `audio`, `video`, `screen`.
    pub media: String,
    #[serde(default)]
    pub sdp: String,
    #[serde(default)]
    pub group: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CallAnswerPayload {
    pub call_id: Uuid,
    #[serde(default)]
    pub accepted: bool,
    #[serde(default)]
    pub sdp: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct IceCandidatePayload {
    pub call_id: Uuid,
    pub candidate: String,
    #[serde(default)]
    pub sdp_mid: Option<String>,
    #[serde(default)]
    pub sdp_index: Option<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CallEndPayload {
    pub call_id: Uuid,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub ended_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CallRejectPayload {
    pub call_id: Uuid,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CallBusyPayload {
    pub call_id: Uuid,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SyncRequestPayload {
    pub chat_id: String,
    #[serde(default)]
    pub since_ms: u64,
    #[serde(default)]
    pub last_message_id: Option<Uuid>,
    #[serde(default)]
    pub limit: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct PingPayload {
    #[serde(default)]
    pub echo_ms: u64,
    #[serde(default)]
    pub nonce: Option<Uuid>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct PongPayload {
    #[serde(default)]
    pub echo_ms: u64,
    #[serde(default)]
    pub nonce: Option<Uuid>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ByePayload {
    #[serde(default)]
    pub reason: Option<String>,
}

/// Why a received envelope was not turned into a usable typed payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IgnoreReason {
    /// The peer used a type this build does not know.
    UnknownType,
    Auth,
    Hello,
    HelloAck,
    Presence,
    ProfileUpdate,
    AvatarRequest,
    AvatarData,
    Message,
    MessageEdit,
    MessageDelete,
    Reaction,
    Receipt,
    Typing,
    PinUpdate,
    GroupState,
    GroupCreate,
    GroupUpdate,
    GroupMemberChange,
    GroupIconUpdate,
    Announcement,
    AnnouncementAck,
    FileOffer,
    FileAccept,
    FileChunk,
    FileAck,
    FileCancel,
    CallOffer,
    CallAnswer,
    IceCandidate,
    CallEnd,
    CallReject,
    CallBusy,
    SyncRequest,
    Ping,
    Pong,
    Bye,
}

/// An ignorable envelope.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ignored {
    pub kind: String,
    pub reason: IgnoreReason,
    pub detail: Option<String>,
}

/// The typed view of any received envelope payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Payload {
    Auth(AuthPayload),
    Hello(HelloPayload),
    HelloAck(HelloAckPayload),
    Presence(PresencePayload),
    ProfileUpdate(ProfileUpdatePayload),
    AvatarRequest(AvatarRequestPayload),
    AvatarData(AvatarDataPayload),
    Message(MessagePayload),
    MessageEdit(MessageEditPayload),
    MessageDelete(MessageDeletePayload),
    Reaction(ReactionPayload),
    Receipt(ReceiptPayload),
    Typing(TypingPayload),
    PinUpdate(PinUpdatePayload),
    GroupState(GroupStatePayload),
    GroupCreate(GroupCreatePayload),
    GroupUpdate(GroupUpdatePayload),
    GroupMemberChange(GroupMemberChangePayload),
    GroupIconUpdate(GroupIconUpdatePayload),
    Announcement(AnnouncementPayload),
    AnnouncementAck(AnnouncementAckPayload),
    FileOffer(FileOfferPayload),
    FileAccept(FileAcceptPayload),
    FileChunk(FileChunkPayload),
    FileAck(FileAckPayload),
    FileCancel(FileCancelPayload),
    CallOffer(CallOfferPayload),
    CallAnswer(CallAnswerPayload),
    IceCandidate(IceCandidatePayload),
    CallEnd(CallEndPayload),
    CallReject(CallRejectPayload),
    CallBusy(CallBusyPayload),
    SyncRequest(SyncRequestPayload),
    Ping(PingPayload),
    Pong(PongPayload),
    Bye(ByePayload),
    /// Unknown types, malformed payloads and out-of-bounds fields land here.
    Ignored(Ignored),
}

impl Payload {
    /// Re-encode this payload to the JSON body it was built from.
    #[must_use]
    pub fn payload_json(&self) -> Option<Value> {
        match self {
            Self::Ignored(_) => None,
            Self::Auth(p) => serde_json::to_value(p).ok(),
            Self::Hello(p) => serde_json::to_value(p).ok(),
            Self::HelloAck(p) => serde_json::to_value(p).ok(),
            Self::Presence(p) => serde_json::to_value(p).ok(),
            Self::ProfileUpdate(p) => serde_json::to_value(p).ok(),
            Self::AvatarRequest(p) => serde_json::to_value(p).ok(),
            Self::AvatarData(p) => serde_json::to_value(p).ok(),
            Self::Message(p) => serde_json::to_value(p).ok(),
            Self::MessageEdit(p) => serde_json::to_value(p).ok(),
            Self::MessageDelete(p) => serde_json::to_value(p).ok(),
            Self::Reaction(p) => serde_json::to_value(p).ok(),
            Self::Receipt(p) => serde_json::to_value(p).ok(),
            Self::Typing(p) => serde_json::to_value(p).ok(),
            Self::PinUpdate(p) => serde_json::to_value(p).ok(),
            Self::GroupState(p) => serde_json::to_value(p).ok(),
            Self::GroupCreate(p) => serde_json::to_value(p).ok(),
            Self::GroupUpdate(p) => serde_json::to_value(p).ok(),
            Self::GroupMemberChange(p) => serde_json::to_value(p).ok(),
            Self::GroupIconUpdate(p) => serde_json::to_value(p).ok(),
            Self::Announcement(p) => serde_json::to_value(p).ok(),
            Self::AnnouncementAck(p) => serde_json::to_value(p).ok(),
            Self::FileOffer(p) => serde_json::to_value(p).ok(),
            Self::FileAccept(p) => serde_json::to_value(p).ok(),
            Self::FileChunk(p) => serde_json::to_value(p).ok(),
            Self::FileAck(p) => serde_json::to_value(p).ok(),
            Self::FileCancel(p) => serde_json::to_value(p).ok(),
            Self::CallOffer(p) => serde_json::to_value(p).ok(),
            Self::CallAnswer(p) => serde_json::to_value(p).ok(),
            Self::IceCandidate(p) => serde_json::to_value(p).ok(),
            Self::CallEnd(p) => serde_json::to_value(p).ok(),
            Self::CallReject(p) => serde_json::to_value(p).ok(),
            Self::CallBusy(p) => serde_json::to_value(p).ok(),
            Self::SyncRequest(p) => serde_json::to_value(p).ok(),
            Self::Ping(p) => serde_json::to_value(p).ok(),
            Self::Pong(p) => serde_json::to_value(p).ok(),
            Self::Bye(p) => serde_json::to_value(p).ok(),
        }
    }

    /// True when the envelope carries nothing this build can act on.
    #[must_use]
    pub fn is_ignored(&self) -> bool {
        matches!(self, Self::Ignored(_))
    }

    /// The envelope type name this payload was classified from; empty when ignored.
    #[must_use]
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Ignored(_) => "",
            Self::Auth(_) => kind::AUTH,
            Self::Hello(_) => kind::HELLO,
            Self::HelloAck(_) => kind::HELLO_ACK,
            Self::Presence(_) => kind::PRESENCE,
            Self::ProfileUpdate(_) => kind::PROFILE_UPDATE,
            Self::AvatarRequest(_) => kind::AVATAR_REQUEST,
            Self::AvatarData(_) => kind::AVATAR_DATA,
            Self::Message(_) => kind::MESSAGE,
            Self::MessageEdit(_) => kind::MESSAGE_EDIT,
            Self::MessageDelete(_) => kind::MESSAGE_DELETE,
            Self::Reaction(_) => kind::REACTION,
            Self::Receipt(_) => kind::RECEIPT,
            Self::Typing(_) => kind::TYPING,
            Self::PinUpdate(_) => kind::PIN_UPDATE,
            Self::GroupState(_) => kind::GROUP_STATE,
            Self::GroupCreate(_) => kind::GROUP_CREATE,
            Self::GroupUpdate(_) => kind::GROUP_UPDATE,
            Self::GroupMemberChange(_) => kind::GROUP_MEMBER_CHANGE,
            Self::GroupIconUpdate(_) => kind::GROUP_ICON_UPDATE,
            Self::Announcement(_) => kind::ANNOUNCEMENT,
            Self::AnnouncementAck(_) => kind::ANNOUNCEMENT_ACK,
            Self::FileOffer(_) => kind::FILE_OFFER,
            Self::FileAccept(_) => kind::FILE_ACCEPT,
            Self::FileChunk(_) => kind::FILE_CHUNK,
            Self::FileAck(_) => kind::FILE_ACK,
            Self::FileCancel(_) => kind::FILE_CANCEL,
            Self::CallOffer(_) => kind::CALL_OFFER,
            Self::CallAnswer(_) => kind::CALL_ANSWER,
            Self::IceCandidate(_) => kind::ICE_CANDIDATE,
            Self::CallEnd(_) => kind::CALL_END,
            Self::CallReject(_) => kind::CALL_REJECT,
            Self::CallBusy(_) => kind::CALL_BUSY,
            Self::SyncRequest(_) => kind::SYNC_REQUEST,
            Self::Ping(_) => kind::PING,
            Self::Pong(_) => kind::PONG,
            Self::Bye(_) => kind::BYE,
        }
    }

    /// The engine subsystem that owns this payload.
    #[must_use]
    pub fn dispatch(&self) -> Dispatch {
        match self {
            Self::Ignored(_) => Dispatch::Ignore,
            Self::Auth(_) | Self::Hello(_) | Self::HelloAck(_) | Self::Bye(_) => {
                Dispatch::Handshake
            }
            Self::Presence(_)
            | Self::ProfileUpdate(_)
            | Self::AvatarRequest(_)
            | Self::AvatarData(_)
            | Self::Typing(_)
            | Self::Ping(_)
            | Self::Pong(_) => Dispatch::Presence,
            Self::Message(_)
            | Self::MessageEdit(_)
            | Self::MessageDelete(_)
            | Self::Reaction(_)
            | Self::Receipt(_)
            | Self::PinUpdate(_)
            | Self::Announcement(_)
            | Self::AnnouncementAck(_) => Dispatch::Conversation,
            Self::GroupState(_)
            | Self::GroupCreate(_)
            | Self::GroupUpdate(_)
            | Self::GroupMemberChange(_)
            | Self::GroupIconUpdate(_) => Dispatch::Groups,
            Self::FileOffer(_)
            | Self::FileAccept(_)
            | Self::FileChunk(_)
            | Self::FileAck(_)
            | Self::FileCancel(_) => Dispatch::Files,
            Self::CallOffer(_)
            | Self::CallAnswer(_)
            | Self::IceCandidate(_)
            | Self::CallEnd(_)
            | Self::CallReject(_)
            | Self::CallBusy(_) => Dispatch::Calls,
            Self::SyncRequest(_) => Dispatch::Sync,
        }
    }

    /// Enforces the documented size limits; oversized payloads are ignorable, not fatal.
    fn size_violation(&self) -> Option<IgnoreReason> {
        match self {
            Self::Message(p)
                if p.text.len() > MAX_BODY_BYTES || p.attachments.len() > MAX_ATTACHMENTS =>
            {
                Some(IgnoreReason::Message)
            }
            Self::MessageEdit(p) if p.body.len() > MAX_BODY_BYTES => {
                Some(IgnoreReason::MessageEdit)
            }
            Self::Announcement(p) if p.text.len() > MAX_BODY_BYTES => {
                Some(IgnoreReason::Announcement)
            }
            Self::Reaction(p) if p.emoji.len() > MAX_REACTION_BYTES => Some(IgnoreReason::Reaction),
            Self::AvatarData(p) if p.bytes.len() > MAX_AVATAR_BYTES => {
                Some(IgnoreReason::AvatarData)
            }
            Self::CallOffer(p) if p.sdp.len() > MAX_SDP_BYTES => Some(IgnoreReason::CallOffer),
            Self::CallAnswer(p) if p.sdp.len() > MAX_SDP_BYTES => Some(IgnoreReason::CallAnswer),
            Self::IceCandidate(p) if p.candidate.len() > MAX_CANDIDATE_BYTES => {
                Some(IgnoreReason::IceCandidate)
            }
            Self::FileChunk(p) if p.bytes.len() > crate::protocol::MAX_FILE_CHUNK_BYTES => {
                Some(IgnoreReason::FileChunk)
            }
            _ => None,
        }
    }
}

/// Convert a decoded envelope into its typed payload.
#[must_use]
pub fn classify(envelope: &Envelope<Value>) -> Payload {
    let value = &envelope.payload;
    let parsed: Result<Payload, IgnoreReason> = match envelope.kind.as_str() {
        kind::AUTH => parse(value, Payload::Auth, IgnoreReason::Auth),
        kind::HELLO => parse(value, Payload::Hello, IgnoreReason::Hello),
        kind::HELLO_ACK => parse(value, Payload::HelloAck, IgnoreReason::HelloAck),
        kind::PRESENCE => parse(value, Payload::Presence, IgnoreReason::Presence),
        kind::PROFILE_UPDATE => parse(value, Payload::ProfileUpdate, IgnoreReason::ProfileUpdate),
        kind::AVATAR_REQUEST => parse(value, Payload::AvatarRequest, IgnoreReason::AvatarRequest),
        kind::AVATAR_DATA => parse(value, Payload::AvatarData, IgnoreReason::AvatarData),
        kind::MESSAGE => parse(value, Payload::Message, IgnoreReason::Message),
        kind::MESSAGE_EDIT => parse(value, Payload::MessageEdit, IgnoreReason::MessageEdit),
        kind::MESSAGE_DELETE => parse(value, Payload::MessageDelete, IgnoreReason::MessageDelete),
        kind::REACTION => parse(value, Payload::Reaction, IgnoreReason::Reaction),
        kind::RECEIPT => parse(value, Payload::Receipt, IgnoreReason::Receipt),
        kind::TYPING => parse(value, Payload::Typing, IgnoreReason::Typing),
        kind::PIN_UPDATE => parse(value, Payload::PinUpdate, IgnoreReason::PinUpdate),
        kind::GROUP_STATE => parse(value, Payload::GroupState, IgnoreReason::GroupState),
        kind::GROUP_CREATE => parse(value, Payload::GroupCreate, IgnoreReason::GroupCreate),
        kind::GROUP_UPDATE => parse(value, Payload::GroupUpdate, IgnoreReason::GroupUpdate),
        kind::GROUP_MEMBER_CHANGE => parse(
            value,
            Payload::GroupMemberChange,
            IgnoreReason::GroupMemberChange,
        ),
        kind::GROUP_ICON_UPDATE => parse(
            value,
            Payload::GroupIconUpdate,
            IgnoreReason::GroupIconUpdate,
        ),
        kind::ANNOUNCEMENT => parse(value, Payload::Announcement, IgnoreReason::Announcement),
        kind::ANNOUNCEMENT_ACK => parse(
            value,
            Payload::AnnouncementAck,
            IgnoreReason::AnnouncementAck,
        ),
        kind::FILE_OFFER => parse(value, Payload::FileOffer, IgnoreReason::FileOffer),
        kind::FILE_ACCEPT => parse(value, Payload::FileAccept, IgnoreReason::FileAccept),
        kind::FILE_CHUNK => parse(value, Payload::FileChunk, IgnoreReason::FileChunk),
        kind::FILE_ACK => parse(value, Payload::FileAck, IgnoreReason::FileAck),
        kind::FILE_CANCEL => parse(value, Payload::FileCancel, IgnoreReason::FileCancel),
        kind::CALL_OFFER => parse(value, Payload::CallOffer, IgnoreReason::CallOffer),
        kind::CALL_ANSWER => parse(value, Payload::CallAnswer, IgnoreReason::CallAnswer),
        kind::ICE_CANDIDATE => parse(value, Payload::IceCandidate, IgnoreReason::IceCandidate),
        kind::CALL_END => parse(value, Payload::CallEnd, IgnoreReason::CallEnd),
        kind::CALL_REJECT => parse(value, Payload::CallReject, IgnoreReason::CallReject),
        kind::CALL_BUSY => parse(value, Payload::CallBusy, IgnoreReason::CallBusy),
        kind::SYNC_REQUEST => parse(value, Payload::SyncRequest, IgnoreReason::SyncRequest),
        kind::PING => parse(value, Payload::Ping, IgnoreReason::Ping),
        kind::PONG => parse(value, Payload::Pong, IgnoreReason::Pong),
        kind::BYE => parse(value, Payload::Bye, IgnoreReason::Bye),
        _ => Err(IgnoreReason::UnknownType),
    };
    parsed
        .map(|payload| match payload.size_violation() {
            Some(reason) => Payload::Ignored(Ignored {
                kind: envelope.kind.clone(),
                reason,
                detail: None,
            }),
            None => payload,
        })
        .unwrap_or_else(|reason| {
            Payload::Ignored(Ignored {
                kind: envelope.kind.clone(),
                reason,
                detail: None,
            })
        })
}

fn parse<T, F>(value: &Value, build: F, reason: IgnoreReason) -> Result<Payload, IgnoreReason>
where
    T: serde::de::DeserializeOwned,
    F: FnOnce(T) -> Payload,
{
    serde_json::from_value::<T>(value.clone())
        .map(build)
        .map_err(|_| reason)
}

/// Route an envelope to the engine subsystem that owns it.
#[must_use]
pub fn dispatch(envelope: &Envelope<Value>) -> Dispatch {
    classify(envelope).dispatch()
}

/// The subsystem that must handle a payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Dispatch {
    /// Session establishment and teardown.
    Handshake,
    /// Presence, profiles, avatars, typing, liveness.
    Presence,
    /// Messages, edits, deletes, reactions, receipts, pins, announcements.
    Conversation,
    /// Signed roster state.
    Groups,
    /// File transfer signalling.
    Files,
    /// Call signalling.
    Calls,
    /// Catch-up requests after a reconnect.
    Sync,
    /// Nothing to do: the frame is dropped without an error.
    Ignore,
}

/// Decode a control frame straight into its typed payload.
///
/// Unknown types, malformed payloads and oversized fields become
/// [`Payload::Ignored`]; only framing-level violations (an oversized or unparseable
/// frame, a wrong protocol version) return `Err`.
pub fn decode_payload_frame(bytes: &[u8]) -> Result<Payload, ProtocolError> {
    if bytes.len() > MAX_CONTROL_FRAME_BYTES {
        return Err(ProtocolError::FrameTooLarge);
    }
    let envelope: Envelope<Value> = serde_json::from_slice(bytes)
        .map_err(|error| ProtocolError::InvalidEnvelope(error.to_string()))?;
    if envelope.v != PROTOCOL_VERSION {
        return Err(ProtocolError::UnsupportedVersion(envelope.v));
    }
    Ok(classify(&envelope))
}

/// True when a frame is small enough to be worth parsing at all.
#[must_use]
pub fn frame_within_limit(bytes: &[u8]) -> bool {
    bytes.len() <= MAX_CONTROL_FRAME_BYTES
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::Identity;
    use crate::protocol::decode_control_frame;

    fn envelope(kind_name: &str, payload: Value) -> Envelope<Value> {
        Envelope {
            v: PROTOCOL_VERSION,
            id: Uuid::now_v7(),
            kind: kind_name.into(),
            from: "peer-a".into(),
            to: "chat-a".into(),
            hlc: Hlc {
                physical_ms: 5,
                logical: 1,
            },
            payload,
        }
    }

    fn group_state() -> GroupState {
        GroupState::create(
            &Identity::from_secret([9; 32]),
            "group-1".into(),
            "Design Team".into(),
            ["peer-b".into()],
        )
        .unwrap()
    }

    fn message_ref() -> MessageRef {
        MessageRef {
            chat_id: "chat-a".into(),
            message_id: Uuid::parse_str("01900000-0000-7000-8000-00000000000a").unwrap(),
        }
    }

    fn uuid(tail: &str) -> Uuid {
        Uuid::parse_str(&format!("01900000-0000-7000-8000-0000000000{tail}")).unwrap()
    }

    fn clock() -> Hlc {
        Hlc {
            physical_ms: 11,
            logical: 2,
        }
    }

    fn attachment() -> MessageAttachment {
        MessageAttachment {
            transfer_id: uuid("ff"),
            file_name: "spec.pdf".into(),
            size: 1024,
            blake3_hash: vec![1; 32],
        }
    }

    /// Every declared type, paired with a representative payload value.
    fn cases() -> Vec<(&'static str, Payload)> {
        let state = group_state();
        vec![
            (
                kind::AUTH,
                Payload::Auth(AuthPayload {
                    nonce: "t9G7Q0Z0m1Vg2w8oQq4hKdLs7nYp3RfBcXaJ0kUeTiM=".into(),
                    mac: Some("kQ2m1Vg2w8oQq4hKdLs7nYp3RfBcXaJ0kUeTiMt9G7Q=".into()),
                }),
            ),
            (
                kind::HELLO,
                Payload::Hello(HelloPayload {
                    peer_id: "peer-a".into(),
                    display_name: "Ada".into(),
                    supported_types: vec![kind::MESSAGE.into()],
                }),
            ),
            (
                kind::HELLO_ACK,
                Payload::HelloAck(HelloAckPayload {
                    peer_id: "peer-a".into(),
                    accepted: true,
                }),
            ),
            (
                kind::PRESENCE,
                Payload::Presence(PresencePayload {
                    peer_id: "peer-a".into(),
                    status: "online".into(),
                    since_ms: 12,
                }),
            ),
            (
                kind::PROFILE_UPDATE,
                Payload::ProfileUpdate(ProfileUpdatePayload {
                    peer_id: "peer-a".into(),
                    display_name: "Ada L".into(),
                    status_text: "at the desk".into(),
                    avatar_hash: Some("ab12".into()),
                    updated_ms: 30,
                }),
            ),
            (
                kind::AVATAR_REQUEST,
                Payload::AvatarRequest(AvatarRequestPayload {
                    peer_id: "peer-a".into(),
                    have_hash: Some("ab12".into()),
                }),
            ),
            (
                kind::AVATAR_DATA,
                Payload::AvatarData(AvatarDataPayload {
                    peer_id: "peer-a".into(),
                    hash: "cd".into(),
                    media_type: "image/png".into(),
                    width: 128,
                    height: 128,
                    bytes: vec![1, 2, 3],
                }),
            ),
            (
                kind::MESSAGE,
                Payload::Message(MessagePayload {
                    chat_id: "chat-a".into(),
                    text: "hello".into(),
                    reply_to: Some(message_ref().message_id),
                    forwarded: false,
                    attachments: vec![attachment()],
                }),
            ),
            (
                kind::MESSAGE_EDIT,
                Payload::MessageEdit(MessageEditPayload {
                    message_id: message_ref().message_id,
                    body: "corrected".into(),
                    clock: clock(),
                }),
            ),
            (
                kind::MESSAGE_DELETE,
                Payload::MessageDelete(MessageDeletePayload {
                    message_id: message_ref().message_id,
                    for_everyone: true,
                    reason: Some("typo".into()),
                }),
            ),
            (
                kind::REACTION,
                Payload::Reaction(ReactionPayload {
                    message: message_ref(),
                    emoji: "thumbs".into(),
                    remove: false,
                }),
            ),
            (
                kind::RECEIPT,
                Payload::Receipt(ReceiptPayload {
                    message: message_ref(),
                    kind: "read".into(),
                    occurred_ms: 44,
                }),
            ),
            (
                kind::TYPING,
                Payload::Typing(TypingPayload {
                    chat_id: "chat-a".into(),
                    typing: true,
                    token: Some(message_ref().message_id),
                }),
            ),
            (
                kind::PIN_UPDATE,
                Payload::PinUpdate(PinUpdatePayload {
                    chat_id: "chat-a".into(),
                    message_id: message_ref().message_id,
                    pinned: true,
                    position: 2,
                }),
            ),
            (
                kind::GROUP_STATE,
                Payload::GroupState(GroupStatePayload(state.clone())),
            ),
            (
                kind::GROUP_CREATE,
                Payload::GroupCreate(GroupCreatePayload {
                    state: state.clone(),
                }),
            ),
            (
                kind::GROUP_UPDATE,
                Payload::GroupUpdate(GroupUpdatePayload {
                    state,
                    note: "renamed".into(),
                }),
            ),
            (
                kind::GROUP_MEMBER_CHANGE,
                Payload::GroupMemberChange(GroupMemberChangePayload {
                    state: group_state(),
                    member: "peer-b".into(),
                    change: "join".into(),
                }),
            ),
            (
                kind::GROUP_ICON_UPDATE,
                Payload::GroupIconUpdate(GroupIconUpdatePayload {
                    group_id: "group-1".into(),
                    icon_color: "#101820".into(),
                    emoji: "palette".into(),
                    epoch: 3,
                }),
            ),
            (
                kind::ANNOUNCEMENT,
                Payload::Announcement(AnnouncementPayload {
                    announcement_id: message_ref().message_id,
                    group_id: "group-1".into(),
                    text: "freeze next monday".into(),
                    created_ms: 50,
                    admins_only: true,
                }),
            ),
            (
                kind::ANNOUNCEMENT_ACK,
                Payload::AnnouncementAck(AnnouncementAckPayload {
                    announcement_id: message_ref().message_id,
                    peer_id: "peer-b".into(),
                    acked_ms: 60,
                }),
            ),
            (
                kind::FILE_OFFER,
                Payload::FileOffer(FileOfferPayload {
                    transfer_id: attachment().transfer_id,
                    chat_id: "chat-a".into(),
                    file_name: "spec.pdf".into(),
                    size: 1024,
                    blake3_hash: vec![1; 32],
                    note: "draft".into(),
                }),
            ),
            (
                kind::FILE_ACCEPT,
                Payload::FileAccept(FileAcceptPayload {
                    transfer_id: attachment().transfer_id,
                    resume_from: 4,
                }),
            ),
            (
                kind::FILE_CHUNK,
                Payload::FileChunk(FileChunkPayload {
                    transfer_id: attachment().transfer_id,
                    index: 4,
                    bytes: vec![7; 16],
                    last: false,
                }),
            ),
            (
                kind::FILE_ACK,
                Payload::FileAck(FileAckPayload {
                    transfer_id: attachment().transfer_id,
                    contiguous_index: 5,
                    complete: false,
                }),
            ),
            (
                kind::FILE_CANCEL,
                Payload::FileCancel(FileCancelPayload {
                    transfer_id: attachment().transfer_id,
                    reason: Some("changed my mind".into()),
                }),
            ),
            (
                kind::CALL_OFFER,
                Payload::CallOffer(CallOfferPayload {
                    call_id: attachment().transfer_id,
                    chat_id: "chat-a".into(),
                    media: "audio".into(),
                    sdp: "v=0".into(),
                    group: false,
                }),
            ),
            (
                kind::CALL_ANSWER,
                Payload::CallAnswer(CallAnswerPayload {
                    call_id: attachment().transfer_id,
                    accepted: true,
                    sdp: "v=0".into(),
                }),
            ),
            (
                kind::ICE_CANDIDATE,
                Payload::IceCandidate(IceCandidatePayload {
                    call_id: attachment().transfer_id,
                    candidate: "candidate:1 1 udp 1 10.0.0.1 5000 typ host".into(),
                    sdp_mid: Some("0".into()),
                    sdp_index: Some(1),
                }),
            ),
            (
                kind::CALL_END,
                Payload::CallEnd(CallEndPayload {
                    call_id: attachment().transfer_id,
                    reason: Some("hangup".into()),
                    ended_ms: 90,
                }),
            ),
            (
                kind::CALL_REJECT,
                Payload::CallReject(CallRejectPayload {
                    call_id: attachment().transfer_id,
                    reason: Some("busy elsewhere".into()),
                }),
            ),
            (
                kind::CALL_BUSY,
                Payload::CallBusy(CallBusyPayload {
                    call_id: attachment().transfer_id,
                }),
            ),
            (
                kind::SYNC_REQUEST,
                Payload::SyncRequest(SyncRequestPayload {
                    chat_id: "chat-a".into(),
                    since_ms: 10,
                    last_message_id: Some(message_ref().message_id),
                    limit: 100,
                }),
            ),
            (
                kind::PING,
                Payload::Ping(PingPayload {
                    echo_ms: 7,
                    nonce: Some(message_ref().message_id),
                }),
            ),
            (
                kind::PONG,
                Payload::Pong(PongPayload {
                    echo_ms: 7,
                    nonce: Some(message_ref().message_id),
                }),
            ),
            (
                kind::BYE,
                Payload::Bye(ByePayload {
                    reason: Some("leaving".into()),
                }),
            ),
        ]
    }

    #[test]
    fn every_declared_type_round_trips_through_the_typed_layer() {
        let cases = cases();
        assert_eq!(
            cases.len(),
            ALL_TYPES.len(),
            "every declared type needs a round-trip case"
        );
        for (kind_name, expected) in &cases {
            let json = expected.payload_json().expect("typed payload re-encodes");
            let decoded = classify(&envelope(kind_name, json.clone()));
            assert_eq!(&decoded, expected, "{kind_name} must classify");
            assert_eq!(&decoded.type_name(), kind_name);
            assert_eq!(
                decoded.payload_json().unwrap(),
                json,
                "{kind_name} must re-encode identically"
            );
            assert!(!decoded.is_ignored());
        }
        for kind_name in ALL_TYPES {
            assert!(
                cases.iter().any(|(declared, _)| declared == kind_name),
                "{kind_name} is missing from the round-trip table"
            );
            assert!(is_known_type(kind_name));
        }
    }

    #[test]
    fn typed_payloads_survive_the_wire_format() {
        for (kind_name, expected) in cases() {
            let json = expected.payload_json().unwrap();
            let bytes = serde_json::to_vec(&envelope(kind_name, json)).unwrap();
            let frame = decode_control_frame(&bytes)
                .unwrap()
                .unwrap_or_else(|| panic!("{kind_name} must be a framing-layer known type"));
            assert_eq!(classify(&frame), expected, "{kind_name}");
            assert_eq!(
                decode_payload_frame(&bytes).unwrap(),
                expected,
                "{kind_name}"
            );
        }
    }

    #[test]
    fn unknown_types_decode_to_an_ignorable_variant() {
        let frame = envelope("quantum_presence", serde_json::json!({"anything": true}));
        assert_eq!(
            classify(&frame),
            Payload::Ignored(Ignored {
                kind: "quantum_presence".into(),
                reason: IgnoreReason::UnknownType,
                detail: None,
            })
        );
        assert_eq!(dispatch(&frame), Dispatch::Ignore);
        assert_eq!(frame.kind, "quantum_presence");
        // Ignorable frames never become an error at either decoding layer.
        let bytes = serde_json::to_vec(&frame).unwrap();
        assert_eq!(decode_control_frame(&bytes).unwrap(), None);
        assert!(matches!(
            decode_payload_frame(&bytes),
            Ok(Payload::Ignored(Ignored {
                reason: IgnoreReason::UnknownType,
                ..
            }))
        ));
    }

    #[test]
    fn a_full_chunk_rides_exactly_one_transport_frame() {
        let chunk = Payload::FileChunk(FileChunkPayload {
            transfer_id: uuid("fd"),
            index: 0,
            bytes: vec![0xaa_u8; crate::protocol::MAX_FILE_CHUNK_BYTES],
            last: true,
        });
        let json = chunk.payload_json().unwrap();
        assert!(
            json["bytes"].is_string(),
            "chunk bytes travel base64-encoded, not as a number array"
        );
        let bytes = serde_json::to_vec(&envelope(kind::FILE_CHUNK, json)).unwrap();
        assert!(
            bytes.len() <= crate::transport::MAX_TRANSPORT_FRAME_BYTES - 16,
            "a full chunk serializes to {} bytes, over the frame limit",
            bytes.len()
        );
        assert_eq!(decode_payload_frame(&bytes).unwrap(), chunk);
    }

    #[test]
    fn the_largest_avatar_rides_one_control_frame() {
        let offer = Payload::AvatarData(AvatarDataPayload {
            peer_id: "peer-a".into(),
            hash: "ab".repeat(64),
            media_type: "image/png".into(),
            width: 512,
            height: 512,
            bytes: vec![0x5a_u8; MAX_AVATAR_BYTES],
        });
        let json = offer.payload_json().unwrap();
        assert!(
            json["bytes"].is_string(),
            "avatar bytes travel base64-encoded, so a number array cannot bloat the frame"
        );
        let bytes = serde_json::to_vec(&envelope(kind::AVATAR_DATA, json)).unwrap();
        assert!(
            bytes.len() <= crate::protocol::MAX_CONTROL_FRAME_BYTES,
            "a full avatar serializes to {} bytes, over the control-frame limit",
            bytes.len()
        );
        assert_eq!(decode_payload_frame(&bytes).unwrap(), offer);
    }

    #[test]
    fn malformed_or_oversized_payloads_are_ignored_not_fatal() {
        let missing = envelope(kind::MESSAGE, serde_json::json!({"chat_id": "c"}));
        assert_eq!(
            classify(&missing),
            Payload::Ignored(Ignored {
                kind: kind::MESSAGE.into(),
                reason: IgnoreReason::Message,
                detail: None,
            })
        );
        let oversized = envelope(
            kind::MESSAGE,
            serde_json::json!({ "chat_id": "c", "text": "x".repeat(MAX_BODY_BYTES + 1) }),
        );
        assert!(classify(&oversized).is_ignored());
        let fat = Payload::FileChunk(FileChunkPayload {
            transfer_id: uuid("fe"),
            index: 0,
            bytes: vec![0_u8; crate::protocol::MAX_FILE_CHUNK_BYTES + 1],
            last: false,
        });
        let oversized_chunk = envelope(
            kind::FILE_CHUNK,
            fat.payload_json().expect("chunk re-encodes"),
        );
        assert_eq!(
            classify(&oversized_chunk),
            Payload::Ignored(Ignored {
                kind: kind::FILE_CHUNK.into(),
                reason: IgnoreReason::FileChunk,
                detail: None,
            })
        );
        let unusable = [
            Value::Null,
            Value::Bool(true),
            Value::Array(vec![]),
            serde_json::json!({ "emoji": "x".repeat(MAX_REACTION_BYTES + 1) }),
        ];
        for bad in &unusable {
            assert!(classify(&envelope(kind::REACTION, bad.clone())).is_ignored());
            assert!(classify(&envelope("brand_new_type", bad.clone())).is_ignored());
            assert_eq!(
                dispatch(&envelope(kind::REACTION, bad.clone())),
                Dispatch::Ignore
            );
        }
    }

    #[test]
    fn dispatch_covers_every_subsystem() {
        let mut seen = std::collections::BTreeSet::new();
        for (kind_name, expected) in cases() {
            let frame = envelope(kind_name, expected.payload_json().unwrap());
            seen.insert(dispatch(&frame));
            assert_eq!(classify(&frame).dispatch(), dispatch(&frame));
        }
        seen.insert(Dispatch::Ignore);
        let expected: std::collections::BTreeSet<Dispatch> = [
            Dispatch::Handshake,
            Dispatch::Presence,
            Dispatch::Conversation,
            Dispatch::Groups,
            Dispatch::Files,
            Dispatch::Calls,
            Dispatch::Sync,
            Dispatch::Ignore,
        ]
        .into_iter()
        .collect();
        assert_eq!(seen, expected);
    }

    #[test]
    fn frame_limit_guard_matches_the_control_cap() {
        assert!(frame_within_limit(&vec![b' '; MAX_CONTROL_FRAME_BYTES]));
        assert!(!frame_within_limit(&vec![
            b' ';
            MAX_CONTROL_FRAME_BYTES + 1
        ]));
        assert_eq!(
            decode_payload_frame(&vec![b'{'; MAX_CONTROL_FRAME_BYTES + 1]),
            Err(ProtocolError::FrameTooLarge)
        );
    }

    #[test]
    fn known_type_table_is_shared_with_the_framing_layer() {
        for kind_name in ALL_TYPES {
            let bytes =
                serde_json::to_vec(&envelope(kind_name, Value::Object(serde_json::Map::new())))
                    .unwrap();
            assert!(
                decode_control_frame(&bytes).unwrap().is_some(),
                "{kind_name} must be a framing-layer known type"
            );
        }
        assert_eq!(
            decode_control_frame(
                &serde_json::to_vec(&envelope(
                    "not_a_type",
                    Value::Object(serde_json::Map::new())
                ))
                .unwrap()
            )
            .unwrap(),
            None
        );
    }
}
