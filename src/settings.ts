import { mount } from "svelte";
import "./app.css";
import Settings from "./Settings.svelte";

// The settings window (#233, D173): a decorated window of its own, opened on
// demand from the library's ⚙ Settings, like prep mode (O5).
mount(Settings, { target: document.getElementById("app")! });
