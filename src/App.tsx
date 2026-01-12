import { useState, useEffect, useRef, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "./App.css";

interface BroadcastConfig {
  multicast_addr: string;
  port: number;
  fps: number;
  quality: number;
  width: number;
  height: number;
  network_mode: "Multicast" | "Broadcast";
}

interface BroadcastStats {
  fps: number;
  bitrate_kbps: number;
  frame_count: number;
  dropped_frames: number;
  cpu_usage: number;
  latency_ms: number;
}

interface FrameData {
  width: number;
  height: number;
  data: string; // Base64 RGBA
  timestamp: number;
  is_keyframe: boolean;
}

type AppMode = "select" | "teacher" | "student";

function App() {
  const [mode, setMode] = useState<AppMode>("select");
  const [config, setConfig] = useState<BroadcastConfig | null>(null);
  const [isRunning, setIsRunning] = useState(false);
  const [stats, setStats] = useState<BroadcastStats | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [frameCount, setFrameCount] = useState(0);
  const [logs, setLogs] = useState<string[]>([]);
  const [showLogs, setShowLogs] = useState(true);
  
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const ctxRef = useRef<CanvasRenderingContext2D | null>(null);
  const logsEndRef = useRef<HTMLDivElement>(null);

  // Load default config
  useEffect(() => {
    invoke<BroadcastConfig>("get_default_config").then(setConfig);
  }, []);

  // Setup canvas context
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

  // Auto scroll logs
  useEffect(() => {
    logsEndRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [logs]);

  // Listen for stats updates (teacher)
  useEffect(() => {
    if (mode !== "teacher" || !isRunning) return;

    const unlisten = listen<BroadcastStats>("broadcast-stats", (event) => {
      setStats(event.payload);
    });

    return () => {
      unlisten.then((fn) => fn());
    };
  }, [mode, isRunning]);

  // Listen for video frames (student)
  useEffect(() => {
    if (mode !== "student" || !isRunning) return;

    const unlisten = listen<FrameData>("video-frame", (event) => {
      renderFrame(event.payload);
      setFrameCount((c) => c + 1);
    });

    return () => {
      unlisten.then((fn) => fn());
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

    // Decode base64 and render
    const binary = atob(frame.data);
    const bytes = new Uint8ClampedArray(binary.length);
    for (let i = 0; i < binary.length; i++) {
      bytes[i] = binary.charCodeAt(i);
    }

    const imageData = new ImageData(bytes, frame.width, frame.height);
    ctx.putImageData(imageData, 0, 0);
  }, []);

  const startTeacher = async () => {
    if (!config) return;
    setError(null);
    await invoke("clear_logs");
    setLogs([]);
    try {
      await invoke("start_teacher_broadcast", { config });
      setIsRunning(true);
    } catch (e) {
      setError(String(e));
    }
  };

  const stopTeacher = async () => {
    try {
      await invoke("stop_teacher_broadcast");
      setIsRunning(false);
      setStats(null);
    } catch (e) {
      setError(String(e));
    }
  };

  const startStudent = async () => {
    if (!config) return;
    setError(null);
    setFrameCount(0);
    await invoke("clear_logs");
    setLogs([]);
    try {
      await invoke("start_student_receiver", { config });
      setIsRunning(true);
    } catch (e) {
      setError(String(e));
    }
  };

  const stopStudent = async () => {
    try {
      await invoke("stop_student_receiver");
      setIsRunning(false);
    } catch (e) {
      setError(String(e));
    }
  };

  const LogPanel = () => (
    <div className="log-panel">
      <div className="log-header">
        <h3>📋 Debug Logs</h3>
        <div className="log-actions">
          <button onClick={() => invoke("clear_logs").then(() => setLogs([]))}>
            Clear
          </button>
          <button onClick={() => setShowLogs(!showLogs)}>
            {showLogs ? "Hide" : "Show"}
          </button>
        </div>
      </div>
      {showLogs && (
        <div className="log-content">
          {logs.length === 0 ? (
            <div className="log-empty">No logs yet...</div>
          ) : (
            logs.map((log, i) => (
              <div key={i} className={`log-line ${log.includes("error") || log.includes("Error") ? "log-error" : ""}`}>
                {log}
              </div>
            ))
          )}
          <div ref={logsEndRef} />
        </div>
      )}
    </div>
  );

  if (mode === "select") {
    return (
      <div className="container mode-select">
        <h1>🖥️ Screen Broadcast</h1>
        <p>UDP Multicast + H.264 cho phòng máy</p>
        
        <div className="mode-buttons">
          <button className="mode-btn teacher" onClick={() => setMode("teacher")}>
            <span className="icon">👨‍🏫</span>
            <span className="label">Teacher</span>
            <span className="desc">Phát màn hình</span>
          </button>
          
          <button className="mode-btn student" onClick={() => setMode("student")}>
            <span className="icon">👨‍🎓</span>
            <span className="label">Student</span>
            <span className="desc">Xem màn hình</span>
          </button>
        </div>
      </div>
    );
  }

  if (mode === "teacher") {
    return (
      <div className="container teacher-mode">
        <header>
          <button className="back-btn" onClick={() => { stopTeacher(); setMode("select"); }}>
            ← Quay lại
          </button>
          <h2>👨‍🏫 Teacher Mode</h2>
        </header>

        {config && (
          <div className="config-panel">
            <h3>Cấu hình</h3>
            <div className="config-grid">
              <label>
                Network Mode:
                <select
                  value={config.network_mode}
                  onChange={(e) => {
                    const mode = e.target.value as "Multicast" | "Broadcast";
                    setConfig({ 
                      ...config, 
                      network_mode: mode,
                      multicast_addr: mode === "Broadcast" ? "255.255.255.255" : "239.255.0.1"
                    });
                  }}
                  disabled={isRunning}
                >
                  <option value="Broadcast">Broadcast (255.255.255.255)</option>
                  <option value="Multicast">Multicast (239.255.0.1)</option>
                </select>
              </label>
              <label>
                Port:
                <input
                  type="number"
                  value={config.port}
                  onChange={(e) => setConfig({ ...config, port: parseInt(e.target.value) })}
                  disabled={isRunning}
                />
              </label>
              <label>
                FPS:
                <input
                  type="range"
                  min="5"
                  max="30"
                  value={config.fps}
                  onChange={(e) => setConfig({ ...config, fps: parseInt(e.target.value) })}
                  disabled={isRunning}
                />
                <span>{config.fps}</span>
              </label>
              <label>
                Quality (QP):
                <input
                  type="range"
                  min="18"
                  max="40"
                  value={config.quality}
                  onChange={(e) => setConfig({ ...config, quality: parseInt(e.target.value) })}
                  disabled={isRunning}
                />
                <span>{config.quality}</span>
              </label>
            </div>
          </div>
        )}

        <div className="controls">
          {!isRunning ? (
            <button className="start-btn" onClick={startTeacher}>
              ▶️ Bắt đầu phát
            </button>
          ) : (
            <button className="stop-btn" onClick={stopTeacher}>
              ⏹️ Dừng phát
            </button>
          )}
        </div>

        {stats && (
          <div className="stats-panel">
            <h3>📊 Thống kê</h3>
            <div className="stats-grid">
              <div className="stat">
                <span className="value">{stats.fps.toFixed(1)}</span>
                <span className="label">FPS</span>
              </div>
              <div className="stat">
                <span className="value">{stats.bitrate_kbps.toFixed(0)}</span>
                <span className="label">Kbps</span>
              </div>
              <div className="stat">
                <span className="value">{stats.frame_count}</span>
                <span className="label">Frames</span>
              </div>
              <div className="stat">
                <span className="value">{stats.latency_ms.toFixed(1)}</span>
                <span className="label">ms</span>
              </div>
            </div>
          </div>
        )}

        <LogPanel />

        {error && <div className="error">{error}</div>}
      </div>
    );
  }

  // Student mode
  return (
    <div className="container student-mode">
      <header>
        <button className="back-btn" onClick={() => { stopStudent(); setMode("select"); }}>
          ← Quay lại
        </button>
        <h2>👨‍🎓 Student Mode</h2>
        {isRunning && <span className="frame-counter">Frames: {frameCount}</span>}
      </header>

      {config && !isRunning && (
        <div className="config-panel">
          <h3>Kết nối</h3>
          <div className="config-grid">
            <label>
              Network Mode:
              <select
                value={config.network_mode}
                onChange={(e) => {
                  const mode = e.target.value as "Multicast" | "Broadcast";
                  setConfig({ 
                    ...config, 
                    network_mode: mode,
                    multicast_addr: mode === "Broadcast" ? "255.255.255.255" : "239.255.0.1"
                  });
                }}
              >
                <option value="Broadcast">Broadcast (255.255.255.255)</option>
                <option value="Multicast">Multicast (239.255.0.1)</option>
              </select>
            </label>
            <label>
              Port:
              <input
                type="number"
                value={config.port}
                onChange={(e) => setConfig({ ...config, port: parseInt(e.target.value) })}
              />
            </label>
          </div>
          <div className="test-buttons">
            <button 
              className="test-btn"
              onClick={async () => {
                await invoke("test_network_info");
              }}
            >
              🔍 Test Network
            </button>
            <button 
              className="test-btn"
              onClick={async () => {
                try {
                  await invoke("test_receive_packet", { config });
                } catch (e) {
                  console.log(e);
                }
              }}
            >
              📡 Test Receive (5s)
            </button>
          </div>
        </div>
      )}

      <div className="controls">
        {!isRunning ? (
          <button className="start-btn" onClick={startStudent}>
            📡 Kết nối
          </button>
        ) : (
          <button className="stop-btn" onClick={stopStudent}>
            ⏹️ Ngắt kết nối
          </button>
        )}
      </div>

      <div className="video-container">
        <canvas ref={canvasRef} className="video-canvas" />
        {!isRunning && (
          <div className="placeholder">
            <span>📺</span>
            <p>Chờ kết nối...</p>
          </div>
        )}
        {isRunning && frameCount === 0 && (
          <div className="placeholder">
            <span>⏳</span>
            <p>Đang chờ stream từ Teacher...</p>
          </div>
        )}
      </div>

      <LogPanel />

      {error && <div className="error">{error}</div>}
    </div>
  );
}

export default App;
