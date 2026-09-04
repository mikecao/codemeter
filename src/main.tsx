import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import "./style.css";

// macOS renders the popup as a frameless, transparent window so its corners
// can be rounded via CSS. Windows and Linux keep their normal opaque framed
// window, so they must NOT get the transparent/rounded treatment.
if (navigator.userAgent.includes("Mac")) {
  document.documentElement.classList.add("macos");
}

createRoot(document.getElementById("app")!).render(
  <StrictMode>
    <App />
  </StrictMode>
);
