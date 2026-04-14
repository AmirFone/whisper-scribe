import { getCurrentWindow } from "@tauri-apps/api/window";

export default function DragHandle() {
  async function startDrag(e: MouseEvent) {
    e.preventDefault();
    await getCurrentWindow().startDragging();
  }

  return (
    <div class="drag-handle" onMouseDown={startDrag}>
      <div class="drag-dots">
        <span /><span /><span /><span /><span />
      </div>
    </div>
  );
}
