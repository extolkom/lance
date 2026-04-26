"use client";

import React, { useRef, useState } from "react";

export interface RichTextEditorProps {
  value?: string;
  onChange?: (html: string) => void;
  id?: string;
}

export function RichTextEditor({ value = "", onChange, id }: RichTextEditorProps) {
  const ref = useRef<HTMLDivElement | null>(null);
  const [isFocused, setIsFocused] = useState(false);

  function exec(command: string, value?: string) {
    document.execCommand(command, false, value || "");
    notifyChange();
    ref.current?.focus();
  }

  function notifyChange() {
    const html = ref.current?.innerHTML ?? "";
    onChange?.(html);
  }

  return (
    <div>
      <div className="mb-2 flex flex-wrap gap-2" role="toolbar" aria-label="Formatting options">
        <button
          type="button"
          aria-label="Bold"
          onClick={() => exec("bold")}
          className="rounded px-2 py-1 text-sm hover:bg-slate-100 focus:outline-none focus:ring-2"
        >
          <strong>B</strong>
        </button>
        <button
          type="button"
          aria-label="Italic"
          onClick={() => exec("italic")}
          className="rounded px-2 py-1 text-sm hover:bg-slate-100 focus:outline-none focus:ring-2"
        >
          <em>I</em>
        </button>
        <button
          type="button"
          aria-label="Insert link"
          onClick={() => {
            const url = window.prompt("Enter URL") || "";
            if (url) exec("createLink", url);
          }}
          className="rounded px-2 py-1 text-sm hover:bg-slate-100 focus:outline-none focus:ring-2"
        >
          Link
        </button>
      </div>

      <div
        id={id}
        ref={ref}
        contentEditable
        suppressContentEditableWarning
        role="textbox"
        aria-multiline="true"
        onInput={notifyChange}
        onBlur={() => setIsFocused(false)}
        onFocus={() => setIsFocused(true)}
        className={`min-h-[160px] w-full rounded-2xl border border-slate-200 bg-slate-50 px-4 py-3 text-slate-950 outline-none transition focus:border-amber-400 ${
          isFocused ? "ring-2 ring-amber-200" : ""
        }`}
        dangerouslySetInnerHTML={{ __html: value }}
      />
    </div>
  );
}

export default RichTextEditor;
