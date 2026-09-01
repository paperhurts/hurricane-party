import { mount } from "svelte";
import "../app.css";
import "./chrome.css";
import Playlist from "./Playlist.svelte";

mount(Playlist, { target: document.getElementById("app")! });
