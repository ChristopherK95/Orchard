import { render } from "solid-js/web";
import { App } from "./App";
import { PopOut } from "./PopOut";
import "@fontsource-variable/geist";
import "@fontsource-variable/geist-mono";
// The wordmark's face (startup panels only), in its one weight.
import "@fontsource/space-grotesk/600.css";
import "./styles.css";

// A popped-out Manual editor window is opened at `index.html#popout` (the core knows its file).
const poppedOut = window.location.hash === "#popout";

render(() => (poppedOut ? <PopOut /> : <App />), document.getElementById("root")!);