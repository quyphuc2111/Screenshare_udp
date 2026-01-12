# 🖥️ Screen Broadcast System

Hệ thống chia sẻ màn hình phòng máy (30-50 client) sử dụng UDP Multicast + H.264 trong Tauri.

## 🎯 Tính năng

- **Teacher App**: Chụp màn hình → Encode H.264 → UDP Multicast
- **Student App**: Join multicast → Decode H.264 → Render realtime
- **Mạng**: LAN nội bộ (offline, không internet)
- **Độ trễ**: ≤ 150ms
- **CPU Teacher**: < 30%
- **CPU Client**: < 10%

## 🏗️ Kiến trúc

```
┌─────────────────┐     UDP Multicast      ┌─────────────────┐
│   Teacher App   │ ──────────────────────▶│  Student Apps   │
│                 │    239.255.0.1:5000    │   (30-50 máy)   │
│ Screen Capture  │                        │                 │
│ H.264 Encode    │                        │ H.264 Decode    │
│ RTP Packetize   │                        │ Render Canvas   │
└─────────────────┘                        └─────────────────┘
```

## 📦 Cài đặt

### Yêu cầu
- Rust 1.70+
- Node.js 18+
- Tauri CLI 2.x

### Build

```bash
# Cài dependencies
npm install

# Build development
npm run tauri dev

# Build production
npm run tauri build
```

## 🚀 Sử dụng

### Teacher (Giáo viên)

1. Mở ứng dụng, chọn **Teacher**
2. Cấu hình:
   - **Multicast IP**: `239.255.0.1` (mặc định)
   - **Port**: `5000`
   - **FPS**: 15-30 (khuyến nghị 15 cho LAN)
   - **Quality**: 28 (thấp hơn = chất lượng cao hơn)
3. Nhấn **Bắt đầu phát**

### Student (Học sinh)

1. Mở ứng dụng, chọn **Student**
2. Nhập cùng **Multicast IP** và **Port** với Teacher
3. Nhấn **Kết nối**

## ⚙️ Cấu hình mạng

### Router/Switch
- Đảm bảo multicast được bật trên switch
- IGMP snooping nên được cấu hình đúng

### Firewall
- Mở port UDP 5000 (hoặc port đã cấu hình)
- Cho phép multicast group 239.255.0.1

### macOS
```bash
# Kiểm tra multicast routing
netstat -rn | grep 239

# Nếu cần, thêm route
sudo route add -net 239.0.0.0/8 -interface en0
```

### Windows
```powershell
# Kiểm tra firewall
netsh advfirewall firewall show rule name=all | findstr "5000"

# Thêm rule nếu cần
netsh advfirewall firewall add rule name="Screen Broadcast" dir=in action=allow protocol=UDP localport=5000
```

## 📊 Thông số kỹ thuật

| Thông số | Giá trị |
|----------|---------|
| Codec | H.264 (OpenH264) |
| Transport | UDP Multicast |
| Multicast Group | 239.255.0.1 |
| Port | 5000 |
| Max Packet Size | 1400 bytes (MTU safe) |
| Keyframe Interval | 2 giây |
| Default FPS | 15 |
| Default Bitrate | ~1.5-3 Mbps (auto) |

## 🔧 Troubleshooting

### Student không nhận được stream
1. Kiểm tra cùng mạng LAN
2. Kiểm tra firewall
3. Kiểm tra multicast routing
4. Thử ping multicast: `ping 239.255.0.1`

### Hình ảnh bị giật
1. Giảm FPS xuống 10-15
2. Tăng Quality (QP) lên 35-40
3. Kiểm tra băng thông mạng

### CPU cao
1. Giảm FPS
2. Giảm độ phân giải màn hình
3. Tăng Quality (QP)

## 📁 Cấu trúc dự án

```
├── src/                    # React frontend
│   ├── App.tsx            # Main component
│   └── App.css            # Styles
├── src-tauri/             # Rust backend
│   ├── src/
│   │   ├── lib.rs         # Tauri entry
│   │   ├── commands.rs    # Tauri commands
│   │   └── broadcast/     # Core modules
│   │       ├── capture.rs # Screen capture
│   │       ├── encoder.rs # H.264 encoding
│   │       ├── network.rs # UDP multicast
│   │       ├── receiver.rs# Stream receiver
│   │       └── types.rs   # Data types
│   └── Cargo.toml
└── package.json
```

## 📝 License

MIT
