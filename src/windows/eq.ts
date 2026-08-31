import { mount } from "svelte";
import "../app.css";
import "./chrome.css";
import Eq from "./Eq.svelte";

mount(Eq, { target: document.getElementById("app")! });
