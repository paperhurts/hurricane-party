import { mount } from "svelte";
import "./app.css";
import Prep from "./Prep.svelte";

// Hurricane Party Planning (#163): a decorated window of its own, opened on
// demand (O5), like the video window.
mount(Prep, { target: document.getElementById("app")! });
