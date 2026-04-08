import { render } from "preact";
import { afterEach, describe, expect, it } from "vitest";
import App from "./App";

let root: HTMLDivElement | null = null;

describe("App", () => {
  afterEach(() => {
    if (root) {
      render(null, root);
      root.remove();
      root = null;
    }
    document.body.innerHTML = "";
  });

  it("renders the scaffold heading", () => {
    root = document.createElement("div");
    document.body.appendChild(root);

    render(<App />, root);

    expect(root.textContent).toContain("Codoxear");
  });
});
