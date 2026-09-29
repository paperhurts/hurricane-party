import { mount } from "svelte";
import "./app.css";
import Visuals from "./Visuals.svelte";

// The visuals window (#167, D169): a decorated window of its own, opened by
// Main's VIS button, like the video window (D13).
mount(Visuals, { target: document.getElementById("app")! });
