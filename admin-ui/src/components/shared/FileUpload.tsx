import { Upload } from "lucide-react";
import type { ReactNode } from "react";
import { useRef, useState } from "react";

import { cn } from "@/shared/lib/cn";

interface FileUploadProps {
  accept?: string;
  multiple?: boolean;
  disabled?: boolean;
  onFiles: (files: File[]) => void;
  label?: ReactNode;
  children?: ReactNode;
  className?: string;
}

export function FileUpload({
  accept = "image/*",
  multiple = false,
  disabled = false,
  onFiles,
  label = "Drop image here or click to upload",
  children,
  className,
}: FileUploadProps) {
  const inputRef = useRef<HTMLInputElement>(null);
  const [dragOver, setDragOver] = useState(false);

  function emit(list: FileList | null) {
    if (!list || list.length === 0) {
      return;
    }
    onFiles(Array.from(list));
  }

  function openPicker() {
    if (disabled) {
      return;
    }
    inputRef.current?.click();
  }

  return (
    <div
      role="button"
      tabIndex={disabled ? -1 : 0}
      aria-disabled={disabled || undefined}
      aria-label={typeof label === "string" ? label : "Upload file"}
      onClick={openPicker}
      onKeyDown={(event) => {
        if (disabled) {
          return;
        }
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          openPicker();
        }
      }}
      onDragOver={(event) => {
        if (disabled) {
          return;
        }
        event.preventDefault();
        setDragOver(true);
      }}
      onDragLeave={() => setDragOver(false)}
      onDrop={(event) => {
        if (disabled) {
          return;
        }
        event.preventDefault();
        setDragOver(false);
        emit(event.dataTransfer.files);
      }}
      className={cn(
        "flex flex-col items-center justify-center gap-2 rounded-md border border-dashed border-edge-md bg-surface-1 px-4 py-6 text-center transition-colors outline-none",
        "hover:border-edge-lg hover:bg-surface-2",
        "focus-visible:border-ring focus-visible:ring-ring/50 focus-visible:ring-[3px]",
        dragOver && "border-cta bg-surface-2",
        disabled && "pointer-events-none opacity-50",
        className,
      )}
    >
      <Upload className="size-5 text-text-mute" />
      <span className="text-caption text-text-mute">{label}</span>
      {children ? (
        <span className="text-caption text-text-faint">{children}</span>
      ) : null}
      <input
        ref={inputRef}
        type="file"
        accept={accept}
        multiple={multiple}
        disabled={disabled}
        hidden
        onChange={(event) => {
          emit(event.target.files);
          event.target.value = "";
        }}
      />
    </div>
  );
}
