# WebRTC + SFU Setup Guide

## Architecture

```
Teacher (Tauri)
  └─ Screen Capture (scrap)
  └─ H.264 Encode (openh264)
  └─ WebRTC Publish
       ↓
SFU Server (Rust Binary)
  └─ Forward RTP packets
       ↓
Students (Tauri)
  └─ WebRTC Receive
  └─ H.264 Decode
  └─ Display on Canvas
```

## Setup Steps

### 1. Build SFU Server

```bash
cd src-tauri
cargo build --release --bin sfu_server
```

### 2. Run SFU Server

```bash
# Run on teacher's machine or dedicated server
./target/release/sfu_server

# Server will listen on:
# - HTTP: http://0.0.0.0:8080
# - WebSocket: ws://0.0.0.0:8080/ws
```

### 3. Configure Tauri App

Update `src-tauri/src/commands.rs` to use WebRTC instead of UDP.

### 4. Build Tauri App

```bash
npm run tauri build
```

## Usage

### Teacher:
1. Start SFU server first
2. Open Teacher app
3. Enter SFU URL: `ws://192.168.1.37:8080/ws`
4. Click "Start Broadcasting"

### Students:
1. Open Student app
2. Enter SFU URL: `ws://192.168.1.37:8080/ws`
3. Click "Start Viewing"
4. Video will appear automatically

## Advantages over UDP Broadcast

✅ **Better for Internet**: Works over internet, not just LAN
✅ **NAT Traversal**: Uses STUN/TURN for firewall traversal
✅ **Scalable**: SFU can handle 100+ students efficiently
✅ **Quality**: Adaptive bitrate, better error recovery
✅ **Security**: DTLS encryption built-in

## Network Requirements

- **LAN**: No special requirements
- **Internet**: 
  - Teacher upload: ~2 Mbps per stream
  - Student download: ~2 Mbps
  - SFU bandwidth: Teacher upload × number of students

## Configuration

### SFU Server Port
Edit `src-tauri/src/bin/sfu_server.rs`:
```rust
let addr = SocketAddr::from(([0, 0, 0, 0], 8080)); // Change port here
```

### Video Quality
Edit teacher config:
```rust
let fps = 15;
let bitrate_kbps = 1500;
```

## Troubleshooting

### Connection Failed
- Check SFU server is running
- Check firewall allows port 8080
- Verify WebSocket URL is correct

### No Video
- Check teacher is broadcasting
- Check browser console for errors
- Verify H.264 codec is supported

### High Latency
- Reduce FPS (15 → 10)
- Reduce bitrate (1500 → 1000)
- Use wired connection instead of WiFi

## Development

### Test SFU Server
```bash
# Terminal 1: Run SFU
cargo run --bin sfu_server

# Terminal 2: Run Teacher
cargo run

# Terminal 3: Run Student
cargo run
```

### Logs
```bash
# Enable debug logs
RUST_LOG=debug cargo run --bin sfu_server
```

## Next Steps

- [ ] Add authentication to SFU
- [ ] Add recording feature
- [ ] Add screen annotation
- [ ] Add audio support
- [ ] Add chat feature
