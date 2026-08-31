import { mount } from "svelte";
import "./app.css";
import Video from "./Video.svelte";

// A separate entry point, not a route. The video window is a real OS window
// (D13: decorated, resizable, outside the bond group), and v0.4 needs the same
// shape for main/eq/playlist — which is why the frontend is plain Vite rather
// than SvelteKit.
mount(Video, { target: document.getElementById("app")! });
