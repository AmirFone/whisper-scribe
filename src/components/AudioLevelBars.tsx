import { createSignal, onMount, onCleanup } from "solid-js";
import { invoke, Channel } from "@tauri-apps/api/core";

export default function AudioLevelBars() {
  const barCount = 7;
  const maxHeight = 32;
  const barWidth = 4;
  const gap = 3;
  const totalWidth = barCount * (barWidth + gap) - gap;

  const [displayLevel, setDisplayLevel] = createSignal(0);
  let targetLevel = 0;
  let rafId = 0;
  let lastTs = 0;

  function animate(ts: number) {
    const dt = lastTs === 0 ? 0.016 : (ts - lastTs) / 1000;
    lastTs = ts;

    setDisplayLevel((prev) => {
      const speed = targetLevel > prev ? 18 : 8;
      return prev + (targetLevel - prev) * (1 - Math.exp(-speed * dt));
    });

    rafId = requestAnimationFrame(animate);
  }

  onMount(async () => {
    rafId = requestAnimationFrame(animate);

    const channel = new Channel<{ level: number }>();
    channel.onmessage = (msg) => {
      targetLevel = msg.level;
    };

    try {
      await invoke("subscribe_audio_level", { channel });
    } catch (_) {
      // Command exited — normal teardown
    }
  });

  onCleanup(() => {
    if (rafId) cancelAnimationFrame(rafId);
  });

  const barHeights = () => {
    const l = Math.min(displayLevel() / 100, 1);
    const center = (barCount - 1) / 2;
    return Array.from({ length: barCount }, (_, i) => {
      const dist = Math.abs(i - center) / center;
      return Math.max(3, l * (1 - dist * 0.4) * maxHeight);
    });
  };

  const barColor = () =>
    displayLevel() > 2 ? "rgba(80, 160, 255, 0.55)" : "rgba(255, 255, 255, 0.06)";

  return (
    <div class="audio-level-container">
      <svg width={totalWidth} height={maxHeight} viewBox={`0 0 ${totalWidth} ${maxHeight}`}>
        {barHeights().map((h, i) => (
          <rect
            x={i * (barWidth + gap)}
            y={maxHeight - h}
            width={barWidth}
            height={h}
            rx={2}
            fill={barColor()}
          />
        ))}
      </svg>
    </div>
  );
}
