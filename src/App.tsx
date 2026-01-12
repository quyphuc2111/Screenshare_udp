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
  
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const ctxRef = useRef<CanvasRenderingContext2D | null>(null);

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
                Multicast IP:
                <input
                  type="text"
                  value={config.multicast_addr}
                  onChange={(e) => setConfig({ ...config, multicast_addr: e.target.value })}
                  disabled={isRunning}
                />
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
              Multicast IP:
              <input
                type="text"
                value={config.multicast_addr}
                onChange={(e) => setConfig({ ...config, multicast_addr: e.target.value })}
              />
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
      </div>

      {error && <div className="error">{error}</div>}
    </div>
  );
}

export default App;
