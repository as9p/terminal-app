import { describe, expect, it, vi } from "vitest";

import { followLeftOption } from "./option-key";

function press(type: "keydown" | "keyup", code: string): void {
  window.dispatchEvent(new KeyboardEvent(type, { code }));
}

describe("left option", () => {
  it("one_set_of_window_listeners_serves_every_view", () => {
    const add = vi.spyOn(window, "addEventListener");
    const remove = vi.spyOn(window, "removeEventListener");
    const first = vi.fn();
    const second = vi.fn();
    const stopFirst = followLeftOption(first);
    const stopSecond = followLeftOption(second);
    expect(add.mock.calls.filter(([type]) => type === "keydown")).toHaveLength(1);
    expect(add.mock.calls.filter(([type]) => type === "keyup")).toHaveLength(1);

    press("keydown", "KeyA");
    expect(first).not.toHaveBeenCalled();
    press("keydown", "AltLeft");
    expect(first).toHaveBeenLastCalledWith(true);
    expect(second).toHaveBeenLastCalledWith(true);
    press("keyup", "AltLeft");
    expect(first).toHaveBeenLastCalledWith(false);
    expect(second).toHaveBeenLastCalledWith(false);
    press("keydown", "AltRight");
    expect(first).toHaveBeenCalledTimes(2);

    press("keydown", "AltLeft");
    window.dispatchEvent(new Event("blur"));
    expect(second).toHaveBeenLastCalledWith(false);

    stopFirst();
    expect(remove).not.toHaveBeenCalled();
    stopSecond();
    expect(remove.mock.calls.filter(([type]) => type === "keydown")).toHaveLength(1);
    press("keydown", "AltLeft");
    expect(second).toHaveBeenCalledTimes(4);
    add.mockRestore();
    remove.mockRestore();
  });
});
