//! WebRTC module for Teacher and Student

pub mod teacher;
pub mod student;
pub mod signaling;

pub use teacher::WebRTCTeacher;
pub use student::WebRTCStudent;
pub use signaling::{SignalMessage, PeerRole};
