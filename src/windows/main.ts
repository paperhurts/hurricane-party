import { mount } from "svelte";
import "../app.css";
import "./chrome.css";
import Main from "./Main.svelte";

// One HTML entry per OS window, not a route. The three classic 275px windows
// are separate top-level OS windows that bond to each other (D12/D41), so they
// cannot share a document — which is the reason the frontend is plain Vite.
mount(Main, { target: document.getElementById("app")! });
