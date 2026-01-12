import { useState, useEffect, useRef, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "./App.css";

interface StreamConfig {
  port: number;
  fps: number;
  quality: number;
  network_mode: "Multicast" | "Broadcast";
}

interface StreamStats {
  fps: number;
  bitrate_kbps: number;
  frame_count: number;
  packets_sent: number;
  packets_lost: number;
  latency_ms: number;
}

interface PeerInfo {
  id: string;
  name: string;
  role: "Teacher" | "Student";
  ip: string;
  stream_port: number;
}

interface FrameData {
  width: number;
  height: number;
  data: string;
}

type AppMode = "select" | "teacher" | "student";

function App() {
  const [mode, setMode] = useState<AppMode>("select");
  const [config, setConfig] = useState<StreamConfig | null>(null);
  const [isRunning, setIsRunning] = useState(false);
  const [stats, setStats] = useState<StreamStats | null>(null);
  const [logs, setLogs] = useState<string[]>([]);
  const [peers, setPeers] = useState<PeerInfo[]>([]);
  const [deviceName, setDeviceName] = useState("My Device");
  const [frameCount, setFrameCount] = useState(0);
  
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const ctxRef = useRef<CanvasRenderingContext2D | null>(null);

  // Load config
  useEffect(() => {
    invoke<StreamConfig>("get_default_config").then(setConfig);
  }, []);

  // Setup canvas
  useEffect(() => {
    if (canvasRef.current) {
      ctxRef.current = canvasRef.current.getContext("2d");
    }
  }, [mode]);

  // Poll logs
  useEffect(() => {
    if (mode === "select") return;
    const interval = setInterval(async () => {
      const newLogs = await invoke<string[]>("get_logs");
      setLogs(newLogs);
    }, 500);
    return () => clearInterval(interval);
  }, [mode]);

  // Poll peers
  useEffect(() => {
    if (mode === "select") return;
    const interval = setInterval(async () => {
      const newPeers = await invoke<PeerInfo[]>("get_discovered_peers");
      setPeers(newPeers);
    }, 2000);
    return () => clearInterval(interval);
  }, [mode]);

  // Discovery announce
  useEffect(() => {
    if (mode === "select" || !isRunning) return;
    const interval = setInterval(() => {
      invoke("discovery_announce").catch(console.error);
    }, 2000);
    return () => clearInterval(interval);
  }, [mode, isRunning]);

  // Listen for stats
  useEffect(() => {
    if (mode !== "teacher" || !isRunning) return;
    const unlisten = listen<StreamStats>("stream-stats", (e) => setStats(e.payload));
    return () => { unlisten.then(fn => fn()); };
  }, [mode, isRunning]);

  // Listen for frames - optimized with requestAnimationFrame
  const pendingFrameRef = useRef<FrameData | null>(null);
  const animationFrameRef = useRef<number | null>(null);

  useEffect(() => {
    if (mode !== "student" || !isRunning) return;
    
    const unlisten = listen<FrameData>("video-frame", (e) => {
      // Store latest frame, don't render immediately
      pendingFrameRef.current = e.payload;
      setFrameCount(c => c + 1);
    });
    
    // Render loop using requestAnimationFrame for smooth 60fps
    const renderLoop = () => {
      if (pendingFrameRef.current) {
        renderFrame(pendingFrameRef.current);
        pendingFrameRef.current = null;
      }
      animationFrameRef.current = requestAnimationFrame(renderLoop);
    };
    animationFrameRef.current = requestAnimationFrame(renderLoop);
    
    return () => { 
      unlisten.then(fn => fn());
      if (animationFrameRef.current) {
        cancelAnimationFrame(animationFrameRef.current);
      }
    };
  }, [mode, isRunning]);

  const renderFrame = useCallback((frame: FrameData) => {
    const canvas = canvasRef.current;
    const ctx = ctxRef.current;
    if (!canvas || !ctx) return;

    // Resize canvas if needed
    if (canvas.width !== frame.width || canvas.height !== frame.height) {
      canvas.width = frame.width;
      canvas.height = frame.height;
    }

    // Optimized base64 decode
    const binary = atob(frame.data);
    const len = binary.length;
    const bytes = new Uint8ClampedArray(len);
    
    // Fast decode loop
    for (let i = 0; i < len; i++) {
      bytes[i] = binary.charCodeAt(i);
    }
    
    const imageData = new ImageData(bytes, frame.width, frame.height);
    ctx.putImageData(imageData, 0, 0);
  }, []);

  const startTeacher = async () => {
    if (!config) return;
    await invoke("clear_logs");
    await invoke("start_discovery", { name: deviceName, isTeacher: true, port: config.port });
    await invoke("start_teacher", { config });
    setIsRunning(true);
  };

  const stopTeacher = async () => {
    await invoke("stop_teacher");
    await invoke("stop_discovery");
    setIsRunning(false);
    setStats(null);
  };

  const startStudent = async () => {
    if (!config) return;
    await invoke("clear_logs");
    setFrameCount(0);
    await invoke("start_discovery", { name: deviceName, isTeacher: false, port: config.port });
    await invoke("start_student", { config });
    setIsRunning(true);
  };

  const stopStudent = async () => {
    await invoke("stop_student");
    await invoke("stop_discovery");
    setIsRunning(false);
  };

  // Mode Selection
  if (mode === "select") {
    return (
      <div className="container mode-select">
        <h1>🖥️ Screen Broadcast</h1>
        <p>RTP + H.264 over UDP</p>
        
        <div className="name-input">
          <label>Device Name:</label>
          <input 
            type="text" 
            value={deviceName} 
            onChange={e => setDeviceName(e.target.value)}
            placeholder="Enter your name"
          />
        </div>
        
        <div className="mode-buttons">
          <button className="mode-btn teacher" onClick={() => setMode("teacher")}>
            <span className="icon">👨‍🏫</span>
            <span className="label">Teacher</span>
            <span className="desc">Broadcast screen</span>
          </button>
          
          <button className="mode-btn student" onClick={() => setMode("student")}>
            <span className="icon">👨‍🎓</span>
            <span className="label">Student</span>
            <span className="desc">View screen</span>
          </button>
        </div>
      </div>
    );
  }

  // Teacher Mode
  if (mode === "teacher") {
    return (
      <div className="container teacher-mode">
        <header>
          <button className="back-btn" onClick={() => { stopTeacher(); setMode("select"); }}>
            ← Back
          </button>
          <h2>👨‍🏫 Teacher: {deviceName}</h2>
        </header>

        {config && (
          <div className="config-panel">
            <div className="config-grid">
              <label>
                Mode:
                <select
                  value={config.network_mode}
                  onChange={e => setConfig({...config, network_mode: e.target.value as any})}
                  disabled={isRunning}
                >
                  <option value="Broadcast">Broadcast</option>
                  <option value="Multicast">Multicast</option>
                </select>
              </label>
              <label>
                Port:
                <input type="number" value={config.port} 
                  onChange={e => setConfig({...config, port: parseInt(e.target.value)})}
                  disabled={isRunning} />
              </label>
              <label>
                FPS: {config.fps}
                <input type="range" min="5" max="30" value={config.fps}
                  onChange={e => setConfig({...config, fps: parseInt(e.target.value)})}
                  disabled={isRunning} />
              </label>
              <label>
                Quality: {config.quality}
                <input type="range" min="18" max="40" value={config.quality}
                  onChange={e => setConfig({...config, quality: parseInt(e.target.value)})}
                  disabled={isRunning} />
              </label>
            </div>
          </div>
        )}

        <div className="controls">
          {!isRunning ? (
            <button className="start-btn" onClick={startTeacher}>▶️ Start Broadcast</button>
          ) : (
            <button className="stop-btn" onClick={stopTeacher}>⏹️ Stop</button>
          )}
        </div>

        {stats && (
          <div className="stats-panel">
            <div className="stats-grid">
              <div className="stat"><span className="value">{stats.fps.toFixed(1)}</span><span className="label">FPS</span></div>
              <div className="stat"><span className="value">{stats.bitrate_kbps.toFixed(0)}</span><span className="label">Kbps</span></div>
              <div className="stat"><span className="value">{stats.frame_count}</span><span className="label">Frames</span></div>
              <div className="stat"><span className="value">{stats.latency_ms.toFixed(1)}</span><span className="label">ms</span></div>
            </div>
          </div>
        )}

        {peers.length > 0 && (
          <div className="peers-panel">
            <h3>👥 Connected Students ({peers.filter(p => p.role === "Student").length})</h3>
            <div className="peers-list">
              {peers.filter(p => p.role === "Student").map(p => (
                <div key={p.id} className="peer-item">
                  <span className="peer-name">{p.name}</span>
                  <span className="peer-ip">{p.ip}</span>
                </div>
              ))}
            </div>
          </div>
        )}

        <LogPanel logs={logs} onClear={() => invoke("clear_logs").then(() => setLogs([]))} />
      </div>
    );
  }

  // Student Mode
  return (
    <div className="container student-mode">
      <header>
        <button className="back-btn" onClick={() => { stopStudent(); setMode("select"); }}>
          ← Back
        </button>
        <h2>👨‍🎓 Student: {deviceName}</h2>
        {isRunning && <span className="frame-counter">Frames: {frameCount}</span>}
      </header>

      {config && !isRunning && (
        <div className="config-panel">
          <div className="config-grid">
            <label>
              Mode:
              <select value={config.network_mode}
                onChange={e => setConfig({...config, network_mode: e.target.value as any})}>
                <option value="Broadcast">Broadcast</option>
                <option value="Multicast">Multicast</option>
              </select>
            </label>
            <label>
              Port:
              <input type="number" value={config.port}
                onChange={e => setConfig({...config, port: parseInt(e.target.value)})} />
            </label>
          </div>
          
          {peers.filter(p => p.role === "Teacher").length > 0 && (
            <div className="teachers-list">
              <h4>📡 Available Teachers:</h4>
              {peers.filter(p => p.role === "Teacher").map(t => (
                <button key={t.id} className="teacher-btn"
                  onClick={() => setConfig({...config, port: t.stream_port})}>
                  {t.name} ({t.ip}:{t.stream_port})
                </button>
              ))}
            </div>
          )}
        </div>
      )}

      <div className="controls">
        {!isRunning ? (
          <button className="start-btn" onClick={startStudent}>📡 Connect</button>
        ) : (
          <button className="stop-btn" onClick={stopStudent}>⏹️ Disconnect</button>
        )}
      </div>

      <div className="video-container">
        <canvas ref={canvasRef} className="video-canvas" />
        {!isRunning && <div className="placeholder"><span>📺</span><p>Waiting...</p></div>}
        {isRunning && frameCount === 0 && <div className="placeholder"><span>⏳</span><p>Waiting for stream...</p></div>}
      </div>

      <LogPanel logs={logs} onClear={() => invoke("clear_logs").then(() => setLogs([]))} />
    </div>
  );
}

function LogPanel({ logs, onClear }: { logs: string[], onClear: () => void }) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [autoScroll, setAutoScroll] = useState(true);
  
  useEffect(() => {
    if (autoScroll && containerRef.current) {
      containerRef.current.scrollTop = containerRef.current.scrollHeight;
    }
  }, [logs, autoScroll]);

  const handleScroll = () => {
    if (containerRef.current) {
      const { scrollTop, scrollHeight, clientHeight } = containerRef.current;
      // Disable auto-scroll if user scrolled up
      setAutoScroll(scrollHeight - scrollTop - clientHeight < 50);
    }
  };
  
  return (
    <div className="log-panel">
      <div className="log-header">
        <h3>📋 Logs ({logs.length})</h3>
        <button onClick={onClear}>Clear</button>
      </div>
      <div 
        className="log-content" 
        ref={containerRef}
        onScroll={handleScroll}
      >
        {logs.map((log, i) => <div key={i} className="log-line">{log}</div>)}
      </div>
    </div>
  );
}

export default App;
