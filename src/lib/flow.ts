// The flow (#167, D169): Geiss and MilkDrop's trick. Each frame is the last
// one drawn back in, zoomed a little, turned a little and fading, and the
// music is drawn over it as a ring, its waveform around a circle, so what
// played a moment ago streams outward in trails. Drawn on the GPU in the
// skin's analyser ramp (D36), so it wears what Main wears.

import { bassOf, Pulse, rgbOf, type VisualsFrame } from "./visualsframe";

const FULL_VS = `#version 300 es
in vec2 pos;
out vec2 uv;
void main() {
  uv = pos * 0.5 + 0.5;
  gl_Position = vec4(pos, 0.0, 1.0);
}`;

// The last frame drawn back in: zoomed, turned, swirled, softened and
// faded. The fade subtracts a hair as well as multiplying, because eight
// bits of a slow multiply round back up and a dim ghost would never go.
const WARP_FS = `#version 300 es
precision highp float;
in vec2 uv;
uniform sampler2D prev;
uniform float zoom;
uniform float rot;
uniform float t;
uniform float decay;
uniform float swirl;
uniform vec2 aspect;
uniform vec2 texel;
out vec4 color;
void main() {
  vec2 p = (uv - 0.5) * aspect;
  float c = cos(rot), s = sin(rot);
  p = mat2(c, s, -s, c) * p / zoom;
  p += swirl * vec2(sin(p.y * 5.0 + t * 0.6), cos(p.x * 5.0 - t * 0.45));
  vec2 q = p / aspect + 0.5;
  if (q.x < 0.0 || q.y < 0.0 || q.x > 1.0 || q.y > 1.0) {
    color = vec4(0.0, 0.0, 0.0, 1.0);
    return;
  }
  vec3 sum = texture(prev, q).rgb * 0.5
    + texture(prev, q + vec2(texel.x, 0.0)).rgb * 0.125
    + texture(prev, q - vec2(texel.x, 0.0)).rgb * 0.125
    + texture(prev, q + vec2(0.0, texel.y)).rgb * 0.125
    + texture(prev, q - vec2(0.0, texel.y)).rgb * 0.125;
  color = vec4(max(sum * decay - 0.004, 0.0), 1.0);
}`;

const COPY_FS = `#version 300 es
precision mediump float;
in vec2 uv;
uniform sampler2D src;
out vec4 color;
void main() {
  color = vec4(texture(src, uv).rgb, 1.0);
}`;

const RING_VS = `#version 300 es
in vec2 pos;
in vec3 col;
uniform vec2 scale;
out vec3 vcol;
void main() {
  vcol = col;
  gl_Position = vec4(pos * scale, 0.0, 1.0);
}`;

const RING_FS = `#version 300 es
precision mediump float;
in vec3 vcol;
uniform float alpha;
out vec4 color;
void main() {
  color = vec4(vcol * alpha, 1.0);
}`;

function compile(gl: WebGL2RenderingContext, vs: string, fs: string): WebGLProgram {
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

type Target = { tex: WebGLTexture; fb: WebGLFramebuffer };

/**
 * How much to lift the waveform so its loudest moment nears full size,
 * whatever the volume: the analyser hears the music after the volume, and
 * at a tenth of it the peak is under two percent. Up to sixteen times;
 * digital silence is exactly 128 and stays flat at any gain.
 */
export function autoGain(peak: number): number {
  return Math.min(16, 0.8 / Math.max(peak, 1e-3));
}

/** Where the ring's colours come from along its length: the ramp, cycling. */
export function rampAt(ramp: [number, number, number][], x: number): [number, number, number] {
  const n = ramp.length;
  if (!n) return [1, 1, 1];
  const f = (((x % n) + n) % n);
  const i = Math.floor(f);
  const k = f - i;
  const a = ramp[i];
  const b = ramp[(i + 1) % n];
  return [a[0] + (b[0] - a[0]) * k, a[1] + (b[1] - a[1]) * k, a[2] + (b[2] - a[2]) * k];
}

export class Flow {
  private gl: WebGL2RenderingContext;
  private warp: WebGLProgram;
  private copy: WebGLProgram;
  private ring: WebGLProgram;
  private quad: WebGLBuffer;
  private ringBuf: WebGLBuffer;
  private targets: [Target, Target] | null = null;
  private cur = 0;
  private w = 0;
  private h = 0;
  private ramp: [number, number, number][] = [[1, 1, 1]];
  private pulse = new Pulse();
  private hue = 0;
  private spin = 1;
  /** The waveform's gain, eased: the analyser hears the music after the
   * volume, and the visual should not shrink when a person turns it down. */
  private gain = 1;
  /** Gentle: short trails, slow turns, no pulses (reduced motion, calm). */
  calm = false;

  constructor(private canvas: HTMLCanvasElement) {
    const gl = canvas.getContext("webgl2", { alpha: false, antialias: false, preserveDrawingBuffer: false });
    if (!gl) throw new Error("WebGL2 is not available");
    this.gl = gl;
    this.warp = compile(gl, FULL_VS, WARP_FS);
    this.copy = compile(gl, FULL_VS, COPY_FS);
    this.ring = compile(gl, RING_VS, RING_FS);
    this.quad = gl.createBuffer()!;
    gl.bindBuffer(gl.ARRAY_BUFFER, this.quad);
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 3, -1, -1, 3]), gl.STATIC_DRAW);
    this.ringBuf = gl.createBuffer()!;
  }

  /** The skin's analyser ramp, as `#rrggbb` strings. */
  setRamp(hexes: string[]): void {
    const ramp = hexes.map(rgbOf).filter((c) => c[0] + c[1] + c[2] > 0.05);
    this.ramp = ramp.length ? ramp : [[1, 1, 1]];
  }

  /** The drawing buffer's size in physical pixels. */
  resize(w: number, h: number): void {
    w = Math.max(1, Math.round(w));
    h = Math.max(1, Math.round(h));
    if (w === this.w && h === this.h) return;
    this.w = w;
    this.h = h;
    this.canvas.width = w;
    this.canvas.height = h;
    const gl = this.gl;
    for (const t of this.targets ?? []) {
      gl.deleteTexture(t.tex);
      gl.deleteFramebuffer(t.fb);
    }
    const make = (): Target => {
      const tex = gl.createTexture()!;
      gl.bindTexture(gl.TEXTURE_2D, tex);
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, w, h, 0, gl.RGBA, gl.UNSIGNED_BYTE, null);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
      const fb = gl.createFramebuffer()!;
      gl.bindFramebuffer(gl.FRAMEBUFFER, fb);
      gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, tex, 0);
      gl.clearColor(0, 0, 0, 1);
      gl.clear(gl.COLOR_BUFFER_BIT);
      return { tex, fb };
    };
    this.targets = [make(), make()];
    this.cur = 0;
  }

  private fullscreen(program: WebGLProgram): void {
    const gl = this.gl;
    const loc = gl.getAttribLocation(program, "pos");
    gl.bindBuffer(gl.ARRAY_BUFFER, this.quad);
    gl.enableVertexAttribArray(loc);
    gl.vertexAttribPointer(loc, 2, gl.FLOAT, false, 0, 0);
    gl.drawArrays(gl.TRIANGLES, 0, 3);
    gl.disableVertexAttribArray(loc);
  }

  /** One frame at `t` seconds, `dt` since the last, from the newest audio frame. */
  render(f: VisualsFrame, t: number, dt: number): void {
    if (!this.targets) return;
    const gl = this.gl;
    const k = Math.min(4, Math.max(0.25, dt * 60)); // in 60ths of a second
    this.gain += (autoGain(f.peak) - this.gain) * Math.min(1, 0.05 * k);
    const bass = bassOf(f);
    const energy = Math.min(1, f.rms * this.gain);
    if (!this.calm && f.beat && this.pulse.beat(t)) {
      this.hue += 5;
      this.spin = -this.spin;
    }
    const p = this.calm ? 0 : this.pulse.value(t);
    this.hue += k * (this.calm ? 0.01 : 0.03 + energy * 0.1);

    const zoom = this.calm ? 1.002 : 1.006 + 0.02 * bass + 0.03 * p;
    const rot = this.calm ? 0.0008 : 0.0025 * Math.sin(t * 0.13) + 0.006 * p * this.spin;
    const decay = this.calm ? 0.9 : 0.955;
    const aspect = this.w >= this.h ? [this.w / this.h, 1] : [1, this.h / this.w];

    // 1. The last frame, warped, into the other target.
    const from = this.targets[this.cur];
    const to = this.targets[1 - this.cur];
    gl.bindFramebuffer(gl.FRAMEBUFFER, to.fb);
    gl.viewport(0, 0, this.w, this.h);
    gl.disable(gl.BLEND);
    gl.useProgram(this.warp);
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, from.tex);
    const u = (n: string) => gl.getUniformLocation(this.warp, n);
    gl.uniform1i(u("prev"), 0);
    gl.uniform1f(u("zoom"), Math.pow(zoom, k));
    gl.uniform1f(u("rot"), rot * k);
    gl.uniform1f(u("t"), t);
    gl.uniform1f(u("decay"), Math.pow(decay, k));
    gl.uniform1f(u("swirl"), (this.calm ? 0.0005 : 0.0015 + 0.002 * energy) * k);
    gl.uniform2f(u("aspect"), aspect[0], aspect[1]);
    gl.uniform2f(u("texel"), 1 / this.w, 1 / this.h);
    this.fullscreen(this.warp);

    // 2. The music, as a ring of its waveform, added on top.
    this.drawRing(f, bass, energy, p);

    // 3. On screen.
    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    gl.viewport(0, 0, this.w, this.h);
    gl.disable(gl.BLEND);
    gl.useProgram(this.copy);
    gl.bindTexture(gl.TEXTURE_2D, to.tex);
    gl.uniform1i(gl.getUniformLocation(this.copy, "src"), 0);
    this.fullscreen(this.copy);
    this.cur = 1 - this.cur;
  }

  private drawRing(f: VisualsFrame, bass: number, energy: number, p: number): void {
    const gl = this.gl;
    const n = f.wave.length;
    const radius = 0.3 + 0.1 * bass + 0.05 * p;
    const amp = 0.12 + 0.25 * energy;
    const width = 0.006 + 0.01 * energy;
    // Two vertices per sample, inside and outside the line: a triangle
    // strip, since WebGL draws lines one pixel wide.
    const data = new Float32Array((n + 1) * 2 * 5);
    let o = 0;
    for (let i = 0; i <= n; i++) {
      const a = (i / n) * Math.PI * 2 + Math.PI / 2;
      const v = Math.max(-1, Math.min(1, ((f.wave[i % n] - 128) / 128) * this.gain));
      const r = radius + amp * v;
      const [cr, cg, cb] = rampAt(this.ramp, this.hue + (i / n) * this.ramp.length);
      for (const side of [-1, 1]) {
        const rr = r + side * width;
        data[o++] = Math.cos(a) * rr;
        data[o++] = Math.sin(a) * rr;
        data[o++] = cr;
        data[o++] = cg;
        data[o++] = cb;
      }
    }
    gl.useProgram(this.ring);
    gl.enable(gl.BLEND);
    gl.blendFunc(gl.ONE, gl.ONE);
    gl.bindBuffer(gl.ARRAY_BUFFER, this.ringBuf);
    gl.bufferData(gl.ARRAY_BUFFER, data, gl.STREAM_DRAW);
    const pos = gl.getAttribLocation(this.ring, "pos");
    const col = gl.getAttribLocation(this.ring, "col");
    gl.enableVertexAttribArray(pos);
    gl.enableVertexAttribArray(col);
    gl.vertexAttribPointer(pos, 2, gl.FLOAT, false, 20, 0);
    gl.vertexAttribPointer(col, 3, gl.FLOAT, false, 20, 8);
    const scale = this.w >= this.h ? [this.h / this.w, 1] : [1, this.w / this.h];
    gl.uniform2f(gl.getUniformLocation(this.ring, "scale"), scale[0], scale[1]);
    gl.uniform1f(gl.getUniformLocation(this.ring, "alpha"), this.calm ? 0.35 : 0.55 + 0.3 * p);
    gl.drawArrays(gl.TRIANGLE_STRIP, 0, (n + 1) * 2);
    gl.disableVertexAttribArray(pos);
    gl.disableVertexAttribArray(col);
  }

  dispose(): void {
    const gl = this.gl;
    for (const t of this.targets ?? []) {
      gl.deleteTexture(t.tex);
      gl.deleteFramebuffer(t.fb);
    }
    gl.deleteBuffer(this.quad);
    gl.deleteBuffer(this.ringBuf);
    gl.deleteProgram(this.warp);
    gl.deleteProgram(this.copy);
    gl.deleteProgram(this.ring);
    this.targets = null;
  }
}
