/** Experimental, bounded paper renderer. Not mounted by production screens.
 * Based on the director-supplied September 12 height-field proof. DOM text owns
 * layout and semantics; one decorative WebGL canvas shades the entire stock.
 */
export const presets = {
  diagnostic: {
    raise: 2.5,
    press: 2,
    bevel: 1.4,
    grain: 0.55,
    azimuth: -135,
    elevation: 32,
    ambient: 0.32,
    contact: 0.72,
    reach: 10,
    pigment: 1,
    variation: 0.42,
    polarity: 1,
  },
  restrained: {
    raise: 0.65,
    press: 0.45,
    bevel: 0.8,
    grain: 0.22,
    azimuth: -135,
    elevation: 42,
    ambient: 0.48,
    contact: 0.4,
    reach: 3,
    pigment: 1,
    variation: 0.3,
    polarity: 1,
  },
};
export type ReliefSettings = typeof presets.diagnostic & {
  dark: number;
  blind: number;
  view: number;
};
export interface ReliefMetrics {
  ready: boolean;
  frames: number;
  builds: number;
  buildMs: number;
  submitMs: number;
  width: number;
  height: number;
  scale: number;
  requestedScale: number;
  retainedBytes: number;
  buildPeakBytes: number;
  renderer: string;
  error?: string;
}
const vertex = `attribute vec2 a_position; varying vec2 v_uv;
void main(){v_uv=a_position*.5+.5;gl_Position=vec4(a_position,0.,1.);}`;
const fragment = `precision highp float;
varying vec2 v_uv;
uniform sampler2D u_map,u_paper;
uniform vec2 u_size;
uniform float u_raise,u_press,u_bevel,u_grain,u_azimuth,u_elevation,u_ambient,u_contact,u_reach,u_pigment,u_variation,u_polarity,u_dark,u_blind,u_view;
// R: crisp coverage, GB: signed distance in CSS pixels, A: signed run strength.
vec4 mapAt(vec2 p){return texture2D(u_map,clamp(p/u_size,vec2(0.),vec2(1.)));}
float distanceAt(vec4 m){return ((m.g*255.*256.+m.b*255.)/65535.*2.-1.)*24.;}
float strengthAt(vec4 m){return (m.a*255.-128.)/127.*u_polarity;}
float fiberAt(vec2 p){return texture2D(u_paper,(p+.5)/256.).r-.5;}
float profileAt(vec4 m){float b=max(.23,u_bevel*max(abs(strengthAt(m)),.23));return smoothstep(-b*.45,b*.8,distanceAt(m));}
float surface(vec2 p){
 vec4 m=mapAt(p);float d=distanceAt(m),s=strengthAt(m),b=max(.23,u_bevel*max(abs(s),.23));
 float shape=profileAt(m),depth=s>0.?u_raise:-u_press;
 // Compression is zero when depth is zero: the uninked zero-height gate is exact.
 float compression=.12*min(abs(depth*s),1.)*shape;
 float glyph=depth*abs(s)*shape;
 float rim=s<0.?.065*u_press*abs(s)*exp(-pow((d+b*.65)/(b*.7),2.)):0.;
 return u_grain*fiberAt(p)*(1.-compression)+glyph+rim;
}
vec3 linear(vec3 c){return mix(c/12.92,pow((c+.055)/1.055,vec3(2.4)),step(vec3(.04045),c));}
vec3 srgb(vec3 c){c=max(c,vec3(0.));return mix(12.92*c,1.055*pow(c,vec3(1./2.4))-.055,step(vec3(.0031308),c));}
void main(){
 vec2 p=vec2(v_uv.x,1.-v_uv.y)*u_size;vec4 m=mapAt(p);float h=surface(p),e=.42;
 vec3 n=normalize(vec3(-(surface(p+vec2(e,0.))-surface(p-vec2(e,0.)))/(2.*e),-(surface(p+vec2(0.,e))-surface(p-vec2(0.,e)))/(2.*e),1.));
 float az=radians(u_azimuth),el=radians(u_elevation);vec3 l=vec3(cos(az)*cos(el),sin(az)*cos(el),sin(el));
 float visibility=1.;
 for(int i=1;i<=12;i++){
  float t=u_reach*float(i)/12.;float obstacle=surface(p+normalize(l.xy)*t)-(h+t*tan(el));
  visibility=min(visibility,1.-u_contact*smoothstep(.05,.4,obstacle));
 }
 if(u_view==1.){gl_FragColor=vec4(vec3(.5+h/10.),1.);return;}
 if(u_view==2.){gl_FragColor=vec4(n*.5+.5,1.);return;}
 if(u_view==3.){gl_FragColor=vec4(vec3(m.r),1.);return;}
 if(u_view==4.){gl_FragColor=vec4(vec3(.5+distanceAt(m)/48.),1.);return;}
 if(u_view==5.){gl_FragColor=vec4(vec3(visibility),1.);return;}
 if(u_view==6.){gl_FragColor=vec4(vec3(profileAt(m)),1.);return;}
 float fiber=fiberAt(p);
 vec3 paper=mix(vec3(.953,.930,.882),vec3(.162,.179,.165),u_dark)*(1.+fiber*.045);
 vec3 ink=mix(vec3(.206,.270,.208),vec3(.899,.868,.782),u_dark)*(1.+fiber*u_variation);
 float coverage=m.r*u_pigment*(1.-u_blind);
 vec3 base=linear(mix(paper,ink,coverage));
 float shade=clamp(u_ambient+(1.-u_ambient)*max(0.,dot(n,l))/max(l.z,.12)*visibility,.24,1.8);
 // Matte pigment. No additive highlight: a flat uninked glyph vanishes completely.
 gl_FragColor=vec4(clamp(srgb(base*shade),0.,1.),1.);
}`;

/** Separable squared Euclidean transform; temporary arrays are released per build. */
function edt(
  mask: Uint8Array,
  w: number,
  h: number,
  inside: boolean,
): Float64Array {
  const a = new Float64Array(w * h),
    n = Math.max(w, h);
  const f = new Float64Array(n),
    d = new Float64Array(n),
    v = new Int32Array(n),
    z = new Float64Array(n + 1);
  for (let i = 0; i < a.length; i++)
    a[i] = mask[i] >= 128 === inside ? 0 : 1e12;
  function line(count: number) {
    let k = 0;
    v[0] = 0;
    z[0] = -Infinity;
    z[1] = Infinity;
    for (let q = 1; q < count; q++) {
      let s: number;
      do {
        s = (f[q] + q * q - (f[v[k]] + v[k] * v[k])) / (2 * (q - v[k]));
        if (s <= z[k]) k--;
        else break;
      } while (k >= 0);
      k++;
      v[k] = q;
      z[k] = s;
      z[k + 1] = Infinity;
    }
    k = 0;
    for (let q = 0; q < count; q++) {
      while (z[k + 1] < q) k++;
      d[q] = (q - v[k]) ** 2 + f[v[k]];
    }
  }
  for (let x = 0; x < w; x++) {
    for (let y = 0; y < h; y++) f[y] = a[y * w + x];
    line(h);
    for (let y = 0; y < h; y++) a[y * w + x] = d[y];
  }
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) f[x] = a[y * w + x];
    line(w);
    for (let x = 0; x < w; x++) a[y * w + x] = d[x];
  }
  return a;
}

/** One authored, periodic micro-height tile; color and height share its coordinates. */
function paperTile(): Uint8Array {
  let seed = 47621;
  const random = () => {
    seed = (1664525 * seed + 1013904223) >>> 0;
    return seed / 4294967296;
  };
  const size = 256,
    out = new Float32Array(size * size);
  for (const [n, weight] of [
    [8, 0.17],
    [32, 0.18],
    [128, 0.1],
    [256, 0.045],
  ]) {
    const grid = Float32Array.from({ length: n * n }, () => random() - 0.5);
    for (let y = 0; y < size; y++)
      for (let x = 0; x < size; x++) {
        const gx = (x * n) / size,
          gy = (y * n) / size,
          ix = Math.floor(gx),
          iy = Math.floor(gy),
          fx = gx - ix,
          fy = gy - iy;
        const sx = fx * fx * (3 - 2 * fx),
          sy = fy * fy * (3 - 2 * fy);
        const at = (xx: number, yy: number) =>
          grid[((yy + n) % n) * n + ((xx + n) % n)];
        out[y * size + x] +=
          weight *
          ((1 - sy) * ((1 - sx) * at(ix, iy) + sx * at(ix + 1, iy)) +
            sy * ((1 - sx) * at(ix, iy + 1) + sx * at(ix + 1, iy + 1)));
      }
  }
  for (let f = 0; f < 6500; f++) {
    const x = random() * size,
      y = random() * size,
      angle = random() * Math.PI * 2,
      len = 1 + random() * 5,
      a = (random() - 0.45) * 0.14;
    for (let t = 0; t < len; t += 0.4)
      out[
        ((Math.floor(y + Math.sin(angle) * t) + size) % size) * size +
          ((Math.floor(x + Math.cos(angle) * t) + size) % size)
      ] += a * 0.36;
  }
  const bytes = new Uint8Array(size * size * 4);
  for (let i = 0; i < out.length; i++) {
    const h = Math.round(255 * Math.max(0, Math.min(1, 0.5 + out[i] * 2.4)));
    bytes.fill(h, i * 4, i * 4 + 3);
    bytes[i * 4 + 3] = 255;
  }
  return bytes;
}

export function createReliefRenderer(
  sheet: HTMLElement,
  canvas: HTMLCanvasElement,
  report: (metrics: ReliefMetrics) => void,
) {
  let gl: WebGLRenderingContext | null = null,
    program: WebGLProgram | null = null;
  let map: WebGLTexture | null = null,
    paper: WebGLTexture | null = null,
    buffer: WebGLBuffer | null = null;
  let contextControl: WEBGL_lose_context | null = null;
  let uniforms: Record<string, WebGLUniformLocation | null> = {};
  let cssW = 0,
    cssH = 0,
    disposed = false,
    frame = 0,
    timer: ReturnType<typeof setTimeout> | undefined;
  let fallback = false,
    settings: ReliefSettings = {
      ...presets.diagnostic,
      dark: 0,
      blind: 0,
      view: 0,
    };
  let signature = "",
    built = false;
  const metrics: ReliefMetrics = {
    ready: false,
    frames: 0,
    builds: 0,
    buildMs: 0,
    submitMs: 0,
    width: 0,
    height: 0,
    scale: 1,
    requestedScale: 1,
    retainedBytes: 0,
    buildPeakBytes: 0,
    renderer: "Native text",
  };
  function fail(error: unknown) {
    sheet.classList.remove("ready");
    metrics.ready = false;
    metrics.error = error instanceof Error ? error.message : String(error);
    report({ ...metrics });
  }
  function init() {
    gl = canvas.getContext("webgl", {
      alpha: false,
      antialias: false,
      depth: false,
      stencil: false,
    });
    if (!gl)
      throw new Error(
        "WebGL unavailable. Native selectable text remains visible.",
      );
    contextControl = gl.getExtension("WEBGL_lose_context");
    const compile = (type: number, source: string) => {
      if (!gl) throw new Error("Renderer unavailable");
      const shader = gl.createShader(type);
      if (!shader) throw new Error("Shader allocation failed");
      gl.shaderSource(shader, source);
      gl.compileShader(shader);
      if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
        const message = gl.getShaderInfoLog(shader);
        gl.deleteShader(shader);
        throw new Error(message || "Shader compilation failed");
      }
      return shader;
    };
    program = gl.createProgram();
    if (!program) throw new Error("Program allocation failed");
    const vs = compile(gl.VERTEX_SHADER, vertex),
      fs = compile(gl.FRAGMENT_SHADER, fragment);
    gl.attachShader(program, vs);
    gl.attachShader(program, fs);
    gl.linkProgram(program);
    gl.deleteShader(vs);
    gl.deleteShader(fs);
    if (!gl.getProgramParameter(program, gl.LINK_STATUS))
      throw new Error(gl.getProgramInfoLog(program) || "Shader link failed");
    gl.useProgram(program);
    buffer = gl.createBuffer();
    gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
    gl.bufferData(
      gl.ARRAY_BUFFER,
      new Float32Array([-1, -1, 1, -1, -1, 1, -1, 1, 1, -1, 1, 1]),
      gl.STATIC_DRAW,
    );
    const a = gl.getAttribLocation(program, "a_position");
    gl.enableVertexAttribArray(a);
    gl.vertexAttribPointer(a, 2, gl.FLOAT, false, 0, 0);
    uniforms = {};
    for (const name of ["size", "map", "paper", ...Object.keys(settings)])
      uniforms[name] = gl.getUniformLocation(program, `u_${name}`);
    map = gl.createTexture();
    paper = gl.createTexture();
    gl.activeTexture(gl.TEXTURE1);
    gl.bindTexture(gl.TEXTURE_2D, paper);
    textureParameters(true);
    gl.texImage2D(
      gl.TEXTURE_2D,
      0,
      gl.RGBA,
      256,
      256,
      0,
      gl.RGBA,
      gl.UNSIGNED_BYTE,
      paperTile(),
    );
    gl.uniform1i(uniforms.paper, 1);
    const info = gl.getExtension("WEBGL_debug_renderer_info");
    metrics.renderer = info
      ? String(gl.getParameter(info.UNMASKED_RENDERER_WEBGL))
      : "WebGL (device details unavailable)";
  }
  function textureParameters(repeat: boolean) {
    if (!gl) return;
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
    gl.texParameteri(
      gl.TEXTURE_2D,
      gl.TEXTURE_WRAP_S,
      repeat ? gl.REPEAT : gl.CLAMP_TO_EDGE,
    );
    gl.texParameteri(
      gl.TEXTURE_2D,
      gl.TEXTURE_WRAP_T,
      repeat ? gl.REPEAT : gl.CLAMP_TO_EDGE,
    );
  }
  function build(force = false) {
    if (!gl || disposed) return;
    const start = performance.now(),
      rect = sheet.getBoundingClientRect();
    const wCSS = sheet.clientWidth,
      hCSS = sheet.clientHeight;
    if (wCSS < 1 || hCSS < 1) throw new Error("Surface has no drawable area");
    const requested = window.devicePixelRatio || 1,
      limit = Number(gl.getParameter(gl.MAX_TEXTURE_SIZE));
    const scale = Math.min(
      requested,
      limit / wCSS,
      limit / hCSS,
      Math.sqrt(6_000_000 / (wCSS * hCSS)),
    );
    const runs = [...sheet.querySelectorAll<HTMLElement>("[data-ink]")].map(
      (span) => {
        const style = getComputedStyle(span),
          r = span.getBoundingClientRect(),
          parent = span.parentElement;
        if (
          !parent ||
          span.children.length ||
          span.getClientRects().length !== 1
        )
          throw new Error("Proof supports plain single-line text only");
        const marker = parent.querySelector(".proof-baseline");
        if (!marker) throw new Error("Native baseline anchor missing");
        const fs = parseFloat(style.fontSize);
        return {
          text: span.textContent || "",
          font: `${style.fontStyle} ${style.fontWeight} ${style.fontSize} ${style.fontFamily}`,
          tracking: style.letterSpacing,
          kerning: style.fontKerning,
          direction: style.direction,
          x: r.left - rect.left - sheet.clientLeft,
          baseline:
            marker.getBoundingClientRect().top - rect.top - sheet.clientTop,
          width: r.width,
          fs,
          mode: Number(
            span.closest<HTMLElement>("[data-mode]")?.dataset.mode || 0,
          ),
        };
      },
    );
    const nextSignature = JSON.stringify([wCSS, hCSS, scale, runs]);
    if (!force && signature === nextSignature) return;
    sheet.classList.remove("ready");
    metrics.ready = false;
    const w = Math.round(wCSS * scale),
      h = Math.round(hCSS * scale),
      count = w * h;
    canvas.width = w;
    canvas.height = h;
    cssW = wCSS;
    cssH = hCSS;
    const off = document.createElement("canvas");
    off.width = w;
    off.height = h;
    const ctx = off.getContext("2d", { willReadFrequently: true });
    if (!ctx) throw new Error("Coverage canvas unavailable");
    ctx.scale(scale, scale);
    ctx.textBaseline = "alphabetic";
    ctx.fillStyle = "#fff";
    const strength = new Uint8Array(count);
    strength.fill(128);
    for (const run of runs) {
      ctx.font = run.font;
      ctx.letterSpacing = run.tracking === "normal" ? "0px" : run.tracking;
      ctx.fontKerning = run.kerning as CanvasFontKerning;
      ctx.direction = run.direction === "rtl" ? "rtl" : "ltr";
      if (ctx.direction !== "ltr")
        throw new Error(
          "Bidirectional shaping is outside this proof; native text retained",
        );
      const mt = ctx.measureText(run.text),
        asc = mt.fontBoundingBoxAscent,
        desc = mt.fontBoundingBoxDescent;
      // Chromium snaps the native alphabetic baseline to the device-pixel grid.
      const y = Math.floor(run.baseline * scale) / scale;
      ctx.fillText(run.text, run.x, y);
      const amount = run.fs >= 35 ? 1 : run.fs >= 18 ? 0.29 : 0.1;
      const x0 = Math.max(0, Math.floor((run.x - 24) * scale)),
        x1 = Math.min(w, Math.ceil((run.x + run.width + 24) * scale));
      const y0 = Math.max(0, Math.floor((y - asc - 18) * scale)),
        y1 = Math.min(h, Math.ceil((y + desc + 18) * scale));
      for (let yy = y0; yy < y1; yy++)
        strength.fill(
          Math.round(128 + run.mode * amount * 127),
          yy * w + x0,
          yy * w + x1,
        );
    }
    const pixels = ctx.getImageData(0, 0, w, h).data,
      coverage = new Uint8Array(count);
    for (let i = 0; i < count; i++) coverage[i] = pixels[i * 4 + 3];
    const toInside = edt(coverage, w, h, true),
      toOutside = edt(coverage, w, h, false),
      data = new Uint8Array(count * 4);
    for (let i = 0; i < count; i++) {
      const c = coverage[i] / 255;
      let distance = Math.sqrt(toOutside[i]) - Math.sqrt(toInside[i]);
      distance += distance > 0 ? -0.5 : 0.5;
      if (c > 0 && c < 1) distance = c - 0.5;
      const value = Math.round(
        (Math.max(-24, Math.min(24, distance / scale)) / 48 + 0.5) * 65535,
      );
      data[i * 4] = coverage[i];
      data[i * 4 + 1] = value >> 8;
      data[i * 4 + 2] = value & 255;
      data[i * 4 + 3] = strength[i];
    }
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, map);
    textureParameters(false);
    gl.texImage2D(
      gl.TEXTURE_2D,
      0,
      gl.RGBA,
      w,
      h,
      0,
      gl.RGBA,
      gl.UNSIGNED_BYTE,
      data,
    );
    gl.uniform1i(uniforms.map, 0);
    built = true;
    signature = nextSignature;
    // Estimates, not measured process/GPU memory; excludes browser swap-chain overhead.
    Object.assign(metrics, {
      builds: metrics.builds + 1,
      buildMs: performance.now() - start,
      width: w,
      height: h,
      scale,
      requestedScale: requested,
      retainedBytes: count * 8 + 256 * 256 * 4,
      buildPeakBytes: count * 50 + 256 * 256 * 4,
    });
  }
  function draw() {
    frame = 0;
    if (!gl || !program || !built || disposed || gl.isContextLost()) return;
    const start = performance.now();
    gl.useProgram(program);
    gl.viewport(0, 0, canvas.width, canvas.height);
    gl.uniform2f(uniforms.size, cssW, cssH);
    for (const [key, value] of Object.entries(settings))
      gl.uniform1f(uniforms[key], value);
    gl.drawArrays(gl.TRIANGLES, 0, 6);
    const error = gl.getError();
    if (error !== gl.NO_ERROR) throw new Error(`WebGL draw failed (${error})`);
    metrics.frames++;
    metrics.submitMs = performance.now() - start;
    metrics.ready = !fallback;
    delete metrics.error;
    sheet.classList.toggle("ready", !fallback);
    report({ ...metrics });
  }
  function render() {
    if (!frame && !disposed)
      frame = requestAnimationFrame(() => {
        try {
          draw();
        } catch (e) {
          fail(e);
        }
      });
  }
  function rebuild() {
    // A changed layout must never show a stale raster in front of its live source.
    sheet.classList.remove("ready");
    clearTimeout(timer);
    timer = setTimeout(() => {
      try {
        build();
        render();
      } catch (e) {
        fail(e);
      }
    }, 80);
  }
  const sizeChanged = () => {
    if (
      sheet.clientWidth !== cssW ||
      sheet.clientHeight !== cssH ||
      (window.devicePixelRatio || 1) !== metrics.requestedScale
    )
      rebuild();
  };
  const resize = new ResizeObserver(sizeChanged);
  const mutation = new MutationObserver(rebuild);
  const lost = (event: Event) => {
    event.preventDefault();
    fail("Graphics context lost; native text restored.");
  };
  const restored = () => {
    try {
      init();
      signature = "";
      build(true);
      render();
    } catch (e) {
      fail(e);
    }
  };
  canvas.addEventListener("webglcontextlost", lost);
  canvas.addEventListener("webglcontextrestored", restored);
  const fontsChanged = () => {
    signature = "";
    rebuild();
  };
  void document.fonts
    .load("500 76px Stationery", "SonoJot")
    .then(async (fonts) => {
      await document.fonts.ready;
      if (disposed) return;
      try {
        if (!fonts.length || !document.fonts.check("500 76px Stationery")) {
          throw new Error("SonoJot font unavailable; native text retained.");
        }
        init();
        build();
        render();
        resize.observe(sheet);
        mutation.observe(sheet, {
          childList: true,
          characterData: true,
          subtree: true,
        });
        window.addEventListener("resize", sizeChanged);
        document.fonts.addEventListener("loadingdone", fontsChanged);
      } catch (e) {
        fail(e);
      }
    })
    .catch(fail);
  return {
    update(next: Partial<ReliefSettings>, native = fallback) {
      settings = { ...settings, ...next };
      fallback = native;
      if (native) sheet.classList.remove("ready");
      render();
    },
    metrics: () => ({ ...metrics }),
    /** Explicit test hook: synchronizes GPU completion. Never used by normal UI. */
    measureCompletion() {
      if (!gl) return null;
      const start = performance.now();
      draw();
      gl.finish();
      return performance.now() - start;
    },
    /** GPU elapsed query, separate from CPU submission; explicit diagnostics only. */
    async measureGpuTime(): Promise<number | null> {
      const context = gl;
      if (!context) return null;
      const ext = context.getExtension("EXT_disjoint_timer_query");
      if (!ext) return null;
      const query = ext.createQueryEXT();
      ext.beginQueryEXT(ext.TIME_ELAPSED_EXT, query);
      draw();
      ext.endQueryEXT(ext.TIME_ELAPSED_EXT);
      for (let attempt = 0; attempt < 60; attempt++) {
        await new Promise((resolve) => setTimeout(resolve, 16));
        if (
          context.isContextLost() ||
          context.getParameter(ext.GPU_DISJOINT_EXT)
        ) {
          ext.deleteQueryEXT(query);
          return null;
        }
        if (ext.getQueryObjectEXT(query, ext.QUERY_RESULT_AVAILABLE_EXT)) {
          const nanoseconds: unknown = ext.getQueryObjectEXT(
            query,
            ext.QUERY_RESULT_EXT,
          );
          ext.deleteQueryEXT(query);
          return typeof nanoseconds === "number"
            ? nanoseconds / 1_000_000
            : null;
        }
      }
      ext.deleteQueryEXT(query);
      return null;
    },
    loseContext() {
      contextControl?.loseContext();
    },
    restoreContext() {
      contextControl?.restoreContext();
    },
    invalidate: rebuild,
    dispose() {
      disposed = true;
      clearTimeout(timer);
      cancelAnimationFrame(frame);
      resize.disconnect();
      mutation.disconnect();
      window.removeEventListener("resize", sizeChanged);
      document.fonts.removeEventListener("loadingdone", fontsChanged);
      canvas.removeEventListener("webglcontextlost", lost);
      canvas.removeEventListener("webglcontextrestored", restored);
      sheet.classList.remove("ready");
      gl?.deleteTexture(map);
      gl?.deleteTexture(paper);
      gl?.deleteBuffer(buffer);
      gl?.deleteProgram(program);
      built = false;
    },
  };
}
