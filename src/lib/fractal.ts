// The fractal (#167, D170): a Julia set, the shape z * z + c leaves behind.
// c walks slowly along the edge of the Mandelbrot set's main cardioid, where
// every such shape is connected and intricate (spirals, dendrites, sea
// horses), the bass breathes the zoom, and the beats turn the colours. Each point is coloured
// by how fast it escapes, through the skin's analyser ramp (D36), so the set
// glows in what Main wears. Drawn on the GPU; at more than about two million
// pixels, at half size and stretched, so full screen stays smooth.

import { compile, drawFull, FULL_VS, fullTriangle, glOf, type Visual } from "./gl";
import { autoGain, bassOf, Pulse, rgbOf, type VisualsFrame } from "./visualsframe";

/** How far inside the cardioid's edge c walks: just inside, the sets stay
 * connected; on it, some come apart into dust. */
export const EDGE = 0.985;

/** The largest ramp the shader takes: a skin's `viscolor` is 24. */
const RAMP_MAX = 24;

/** Past this many pixels the set is drawn at half size and stretched. */
const HALF_ABOVE = 2_100_000;

/** The main cardioid's edge at `angle`: e^(i angle) / 2 - e^(2i angle) / 4. */
export function cardioid(angle: number): [number, number] {
  return [
    Math.cos(angle) / 2 - Math.cos(2 * angle) / 4,
    Math.sin(angle) / 2 - Math.sin(2 * angle) / 4,
  ];
}

/** Where c is at `angle`: a little inside the cardioid's edge, drawn in
 * toward zero, which lies inside it. */
export function juliaC(angle: number): [number, number] {
  const [x, y] = cardioid(angle);
  return [x * EDGE, y * EDGE];
}

const JULIA_FS = `#version 300 es
precision highp float;
in vec2 uv;
uniform vec2 c;
uniform float scale;
uniform float rot;
uniform vec2 aspect;
uniform float hue;
uniform float glow;
uniform vec3 ramp[${RAMP_MAX}];
uniform int rampN;
out vec4 color;

vec3 rampAt(float x) {
  float n = float(rampN);
  float f = mod(x, n);
  int i = int(floor(f));
  int j = i + 1 >= rampN ? 0 : i + 1;
  return mix(ramp[i], ramp[j], fract(f));
}

void main() {
  vec2 p = (uv - 0.5) * aspect * 2.0 * scale;
  float cr = cos(rot), sr = sin(rot);
  vec2 z = mat2(cr, sr, -sr, cr) * p;
  const int MAX = 160;
  int n = 0;
  float r2 = 0.0;
  for (int i = 0; i < MAX; i++) {
    z = vec2(z.x * z.x - z.y * z.y, 2.0 * z.x * z.y) + c;
    r2 = dot(z, z);
    if (r2 > 256.0) break;
    n++;
  }
  if (n >= MAX) {
    // Inside the set: a dim wash of the ramp by where the orbit settled,
    // so the heart of it is not a flat hole.
    color = vec4(rampAt(hue + 12.0 + length(z) * 6.0) * 0.16 * glow, 1.0);
    return;
  }
  // The smooth count, so there are no bands between one step and the next,
  // and the ramp spread by its logarithm, so the colours near the edge,
  // where the count climbs fast, are not squeezed into one.
  float mu = float(n) + 1.0 - log2(max(log(r2) * 0.5, 1e-6));
  vec3 col = rampAt(log2(mu + 1.0) * 4.0 + hue);
  float shade = clamp(mu / 16.0, 0.1, 1.0);
  color = vec4(col * shade * glow, 1.0);
}`;

const COPY_FS = `#version 300 es
precision mediump float;
in vec2 uv;
uniform sampler2D src;
out vec4 color;
void main() {
  color = vec4(texture(src, uv).rgb, 1.0);
}`;

type Low = { tex: WebGLTexture; fb: WebGLFramebuffer; w: number; h: number };

export class Fractal implements Visual {
  calm = false;
  private gl: WebGL2RenderingContext;
  private julia: WebGLProgram;
  private copy: WebGLProgram;
  private tri: WebGLBuffer;
  private ramp = new Float32Array(RAMP_MAX * 3).fill(1);
  private rampN = 1;
  private w = 0;
  private h = 0;
  private low: Low | null = null;
  private pulse = new Pulse();
  private angle = 2.2;
  private hue = 0;
  private gain = 1;

  constructor(canvas: HTMLCanvasElement) {
    const gl = glOf(canvas);
    this.gl = gl;
    this.julia = compile(gl, FULL_VS, JULIA_FS);
    this.copy = compile(gl, FULL_VS, COPY_FS);
    this.tri = fullTriangle(gl);
  }

  setRamp(hexes: string[]): void {
    const cols = hexes.map(rgbOf).filter((c) => c[0] + c[1] + c[2] > 0.05).slice(0, RAMP_MAX);
    const use = cols.length ? cols : [[1, 1, 1] as [number, number, number]];
    this.ramp.fill(0);
    use.forEach((c, i) => this.ramp.set(c, i * 3));
    this.rampN = use.length;
  }

  resize(w: number, h: number): void {
    w = Math.max(1, Math.round(w));
    h = Math.max(1, Math.round(h));
    if (w === this.w && h === this.h) return;
    this.w = w;
    this.h = h;
    const gl = this.gl;
    if (this.low) {
      gl.deleteTexture(this.low.tex);
      gl.deleteFramebuffer(this.low.fb);
      this.low = null;
    }
    if (w * h > HALF_ABOVE) {
      const lw = Math.ceil(w / 2);
      const lh = Math.ceil(h / 2);
      const tex = gl.createTexture()!;
      gl.bindTexture(gl.TEXTURE_2D, tex);
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, lw, lh, 0, gl.RGBA, gl.UNSIGNED_BYTE, null);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
      const fb = gl.createFramebuffer()!;
      gl.bindFramebuffer(gl.FRAMEBUFFER, fb);
      gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, tex, 0);
      this.low = { tex, fb, w: lw, h: lh };
    }
  }

  render(f: VisualsFrame, t: number, dt: number): void {
    if (!this.w) return;
    const gl = this.gl;
    const k = Math.min(4, Math.max(0.25, dt * 60)); // in 60ths of a second
    this.gain += (autoGain(f.peak) - this.gain) * Math.min(1, 0.05 * k);
    const bass = bassOf(f);
    const energy = Math.min(1, f.rms * this.gain);
    if (!this.calm && f.beat && this.pulse.beat(t)) this.hue += 3;
    const p = this.calm ? 0 : this.pulse.value(t);
    // Round the cardioid every few minutes at rest, faster as the music fills.
    this.angle += k * (this.calm ? 0.0002 : 0.0006 + 0.002 * energy);
    this.hue += k * (this.calm ? 0.005 : 0.02 + 0.05 * energy);
    const [cx, cy] = juliaC(this.angle + (this.calm ? 0 : 0.02 * bass));
    const scale = this.calm ? 1.55 : 1.55 / (1 + 0.12 * bass + 0.08 * p);
    const aspect = this.w >= this.h ? [this.w / this.h, 1] : [1, this.h / this.w];

    const low = this.low;
    gl.bindFramebuffer(gl.FRAMEBUFFER, low ? low.fb : null);
    gl.viewport(0, 0, low ? low.w : this.w, low ? low.h : this.h);
    gl.disable(gl.BLEND);
    gl.useProgram(this.julia);
    const u = (n: string) => gl.getUniformLocation(this.julia, n);
    gl.uniform2f(u("c"), cx, cy);
    gl.uniform1f(u("scale"), scale);
    gl.uniform1f(u("rot"), t * (this.calm ? 0.005 : 0.02));
    gl.uniform2f(u("aspect"), aspect[0], aspect[1]);
    gl.uniform1f(u("hue"), this.hue);
    gl.uniform1f(u("glow"), this.calm ? 0.8 : 0.9 + 0.25 * p);
    gl.uniform3fv(u("ramp"), this.ramp);
    gl.uniform1i(u("rampN"), this.rampN);
    drawFull(gl, this.julia, this.tri);

    if (low) {
      gl.bindFramebuffer(gl.FRAMEBUFFER, null);
      gl.viewport(0, 0, this.w, this.h);
      gl.useProgram(this.copy);
      gl.activeTexture(gl.TEXTURE0);
      gl.bindTexture(gl.TEXTURE_2D, low.tex);
      gl.uniform1i(gl.getUniformLocation(this.copy, "src"), 0);
      drawFull(gl, this.copy, this.tri);
    }
  }

  dispose(): void {
    const gl = this.gl;
    if (this.low) {
      gl.deleteTexture(this.low.tex);
      gl.deleteFramebuffer(this.low.fb);
      this.low = null;
    }
    gl.deleteBuffer(this.tri);
    gl.deleteProgram(this.julia);
    gl.deleteProgram(this.copy);
  }
}
