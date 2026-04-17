import type { JSX } from "solid-js";

export type ViewMode = "transcription" | "screen";

interface ModeToggleProps {
  mode: ViewMode;
  onModeChange: (mode: ViewMode) => void;
}

export default function ModeToggle(props: ModeToggleProps): JSX.Element {
  return (
    <div class="mode-toggle">
      <button
        class={props.mode === "transcription" ? "active" : ""}
        onClick={() => props.onModeChange("transcription")}
      >
        Transcription
      </button>
      <button
        class={props.mode === "screen" ? "active" : ""}
        onClick={() => props.onModeChange("screen")}
      >
        Screen Context
      </button>
    </div>
  );
}
