import { useCallback, useRef } from "react";

interface ResizeDividerProps {
  onResize: (delta: number) => void;
  direction?: "horizontal" | "vertical";
}

export default function ResizeDivider({
  onResize,
  direction = "horizontal",
}: ResizeDividerProps) {
  const dragging = useRef(false);

  const handleMouseDown = useCallback(
    (e: React.MouseEvent) => {
      e.preventDefault();
      dragging.current = true;
      document.body.classList.add("select-none");

      const startPos =
        direction === "horizontal" ? e.clientX : e.clientY;
      let lastPos = startPos;

      const onMouseMove = (ev: MouseEvent) => {
        const currentPos =
          direction === "horizontal" ? ev.clientX : ev.clientY;
        const delta = currentPos - lastPos;
        lastPos = currentPos;
        onResize(delta);
      };

      const onMouseUp = () => {
        dragging.current = false;
        document.body.classList.remove("select-none");
        document.removeEventListener("mousemove", onMouseMove);
        document.removeEventListener("mouseup", onMouseUp);
      };

      document.addEventListener("mousemove", onMouseMove);
      document.addEventListener("mouseup", onMouseUp);
    },
    [onResize, direction],
  );

  const isHorizontal = direction === "horizontal";

  return (
    <div
      onMouseDown={handleMouseDown}
      className={`shrink-0 group transition-all
        ${
          isHorizontal
            ? "w-[1px] hover:w-[4px] cursor-col-resize"
            : "h-[1px] hover:h-[4px] cursor-row-resize"
        }
        bg-nx-border-light hover:bg-nx-accent/30`}
    >
      <div
        className={`hidden group-hover:flex items-center justify-center h-full w-full
          ${isHorizontal ? "flex-col gap-1" : "flex-row gap-1"}`}
      >
        <div className="w-1 h-1 rounded-full bg-nx-accent/50" />
        <div className="w-1 h-1 rounded-full bg-nx-accent/50" />
        <div className="w-1 h-1 rounded-full bg-nx-accent/50" />
      </div>
    </div>
  );
}
