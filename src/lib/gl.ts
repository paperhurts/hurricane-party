// What the visuals window's styles share (#167): one WebGL 2 context on the
// window's canvas, a triangle that covers the screen, and the shape a style
// has so the window can switch between them.

import type { VisualsFrame } from "./visualsframe";

/** A style the visuals window can show. */
export interface Visual {
  /** Gentle: calm (D130) or reduced motion. */
  calm: boolean;
  /** The skin's analyser ramp, as `#rrggbb` strings. */
  setRamp(hexes: string[]): void;
  /** The canvas's drawing buffer is now `w` x `h` physical pixels. */
  resize(w: number, h: number): void;
  /** One frame at `t` seconds, `dt` since the last, from the newest audio frame. */
  render(f: VisualsFrame, t: number, dt: number): void;
  dispose(): void;
}

/** The canvas's one context. Every style asks the same way, so they share it. */
export function glOf(canvas: HTMLCanvasElement): WebGL2RenderingContext {
  const gl = canvas.getContext("webgl2", { alpha: false, antialias: false, preserveDrawingBuffer: false });
  if (!gl) throw new Error("WebGL2 is not available");
  return gl;
}

/** A vertex shader for a triangle that covers the screen, with `uv` 0..1 across it. */
export const FULL_VS = `#version 300 es
in vec2 pos;
out vec2 uv;
void main() {
  uv = pos * 0.5 + 0.5;
  gl_Position = vec4(pos, 0.0, 1.0);
}`;

export function compile(gl: WebGL2RenderingContext, vs: string, fs: string): WebGLProgram {
  const shader = (type: number, src: string) => {
    const s = gl.createShader(type)!;
    gl.shaderSource(s, src);
    gl.compileShader(s);
    if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(s) ?? "shader");
    return s;
  };
  const p = gl.createProgram()!;
  gl.attachShader(p, shader(gl.VERTEX_SHADER, vs));
  gl.attachShader(p, shader(gl.FRAGMENT_SHADER, fs));
  gl.linkProgram(p);
  if (!gl.getProgramParameter(p, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(p) ?? "program");
  return p;
}

/** The screen-covering triangle's vertices, for `drawFull`. */
export function fullTriangle(gl: WebGL2RenderingContext): WebGLBuffer {
  const b = gl.createBuffer()!;
  gl.bindBuffer(gl.ARRAY_BUFFER, b);
  gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 3, -1, -1, 3]), gl.STATIC_DRAW);
  return b;
}

/** Run `program` over the whole target with `FULL_VS`. */
export function drawFull(gl: WebGL2RenderingContext, program: WebGLProgram, tri: WebGLBuffer): void {
  const loc = gl.getAttribLocation(program, "pos");
  gl.bindBuffer(gl.ARRAY_BUFFER, tri);
  gl.enableVertexAttribArray(loc);
  gl.vertexAttribPointer(loc, 2, gl.FLOAT, false, 0, 0);
  gl.drawArrays(gl.TRIANGLES, 0, 3);
  gl.disableVertexAttribArray(loc);
}
