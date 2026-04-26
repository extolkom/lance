import { render, fireEvent } from "@testing-library/react";
import { describe, it, expect, vi } from "vitest";
import RichTextEditor from "../rich-text-editor";

describe("RichTextEditor", () => {
  it("renders and updates content", () => {
    const handleChange = vi.fn();
    const { getByRole } = render(<RichTextEditor value="<p>hi</p>" onChange={handleChange} />);
    const textbox = getByRole("textbox");
    fireEvent.input(textbox, { target: { innerHTML: "<p>hello</p>" } });
    expect(handleChange).toHaveBeenCalled();
  });
});
