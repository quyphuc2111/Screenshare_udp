# 🚀 Quick Start Guide

## WebRTC Mode (Recommended)

### 1. Start SFU Server (Teacher's Machine)

```bash
cd src-tauri
cargo run --release --bin sfu_server
```

You'll see:
```
🚀 Starting SFU Server...
📡 SFU listening on http://0.0.0.0:8080
WebSocket endpoint: ws://0.0.0.0:8080/ws
```

### 2. Start Teacher App

```bash
npm run tauri dev
```

1. Select **🌐 WebRTC (Internet)** mode
2. Enter SFU URL: `ws://192.168.1.37:8080/ws` (replace with your IP)
3. Click **Teacher** button
4. Click **Start Broadcasting**

### 3. Start Student Apps

```bash
npm run tauri dev
```

1. Select **🌐 WebRTC (Internet)** mode
2. Enter same SFU URL: `ws://192.168.1.37:8080/ws`
3. Click **Student** button
4. Click **Start Viewing**
5. Video appears automatically! 🎉

---

## UDP Mode (LAN Only)

### 1. Start Teacher App

```bash
npm run tauri dev
```

1. Select **📡 UDP (LAN Only)** mode
2. Enter device name
3. Click **Teacher** button
4. Configure settings (optional)
5. Click **Start Broadcasting**

### 2. Start Student Apps

```bash
npm run tauri dev
```

1. Select **📡 UDP (LAN Only)** mode
2. Enter device name
3. Click **Student** button
4. Wait for teacher to appear in list
5. Click **Start Viewing**

---

## Troubleshooting

### WebRTC: Can't connect to SFU

**Check:**
- SFU server is running
- Firewall allows port 8080
- URL is correct (ws:// not wss://)
- IP address is correct

**Fix:**
```bash
# Check if port is in use
lsof -i :8080

# Test SFU health
curl http://192.168.1.37:8080/health
```

### UDP: No video appears

**Check:**
- Both on same network
- Firewall disabled or port 5000 allowed
- Teacher is broadcasting

**Fix:**
```bash
# Test UDP connectivity
# On teacher machine:
./src-tauri/target/debug/test_sender

# On student machine:
./src-tauri/target/debug/test_receiver
```

### Low FPS / Laggy

**Solutions:**
1. Reduce FPS: 15 → 10
2. Reduce quality: 30 → 25
3. Use wired connection
4. Close other apps
5. Use WebRTC mode (better performance)

---

## Performance Tips

### For Best Quality:
- Use WebRTC mode
- Wired ethernet connection
- Dedicated SFU server
- FPS: 15, Quality: 30

### For Low Bandwidth:
- FPS: 10, Quality: 25
- Use UDP mode (less overhead)
- Reduce resolution (edit code)

### For Many Students (30+):
- Use WebRTC + SFU
- Dedicated server for SFU
- Gigabit network
- Monitor CPU usage

---

## Next Steps

- ✅ Test with 2-3 students first
- ✅ Adjust settings for your network
- ✅ Deploy SFU on dedicated server
- ✅ Add authentication (production)
- ✅ Enable HTTPS/WSS (production)

---

**Need Help?** Check `WEBRTC_USAGE.md` for detailed documentation.
