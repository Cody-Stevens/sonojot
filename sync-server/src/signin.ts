import { Fault } from "./common";

const css = `@font-face{font-family:Source Serif;src:url('/assets/source-serif.woff2') format('woff2');font-weight:200 900;font-display:swap}
:root{font:18px/1.5 'Source Serif',Georgia,serif;color:#244437;background:#e9e6dd;color-scheme:light}*{box-sizing:border-box}body{margin:0;padding:clamp(24px,8vh,90px) 18px}main{max-width:540px;margin:auto;padding:clamp(27px,6vw,54px);border-radius:14px;background:#f3f0e7;box-shadow:inset 0 1px #fff9,0 1px 2px #273e3033,0 9px 24px #273e3014;position:relative}main:before{content:'';position:absolute;inset:0;border-radius:inherit;pointer-events:none;background:url('/assets/paper.png') repeat;opacity:.13;background-size:512px}main>*{position:relative}.brand{display:block;width:160px;height:auto;margin-bottom:36px}h1{font-size:2rem;line-height:1.15;font-weight:550;letter-spacing:-.025em;margin:0 0 18px;text-shadow:0 1px 0 #fff9}p{margin:0 0 27px}.muted{color:#526358;font-size:.9rem}form{display:grid;gap:18px}label{display:grid;gap:9px}input,button{font:inherit;color:inherit;border:1px solid #24443736;border-radius:7px;padding:12px 14px;width:100%;min-width:0}input{appearance:none;background:#e9e6dd;box-shadow:inset 0 1px 3px #2444371f}button{cursor:pointer;background:#f3f0e7;box-shadow:inset 0 1px #fff9,0 1px 2px #24443735;transition:background 140ms}button:hover{background:#e5e7dc}button:active{box-shadow:inset 0 1px 3px #24443730}button:disabled{opacity:.55;cursor:wait}a{color:inherit;text-underline-offset:4px}a:focus-visible,input:focus-visible,button:focus-visible{outline:3px solid #527c66;outline-offset:3px}#status{margin:18px 0;overflow-wrap:anywhere}#code{letter-spacing:.25em;font-variant-numeric:tabular-nums}footer{margin-top:27px;border-top:1px solid #24443726;padding-top:18px}footer p{margin:0}.back{display:inline-block;margin-top:18px}[hidden]{display:none!important}@media(prefers-reduced-motion:reduce){*{transition:none!important}}`;

const js = String.raw`'use strict';
const el=id=>document.getElementById(id),signup=location.pathname==='/sign-up',params=new URLSearchParams(location.search),request=params.get('request'),status=el('status');
let pending=request;
if(pending){let web=false;try{web=JSON.parse(sessionStorage.getItem('sj_pending')||'null')?.id===pending}catch{}if(!web){const back=document.querySelector('.back');back.removeAttribute('href');back.textContent='To start again, return to SonoJot and choose Set up sync or Add this computer.'}}
const b64=b=>btoa(String.fromCharCode(...b)).replaceAll('+','-').replaceAll('/','_').replaceAll('=','');
async function api(path,data){const r=await fetch(path,{method:'POST',credentials:'same-origin',headers:{'content-type':'application/json'},body:JSON.stringify(data),redirect:'error'});let v;try{v=await r.json()}catch{throw Error('Sign-in could not connect. Try again.')}if(!r.ok)throw Error(v.error||'Sign-in could not finish. Try again.');return v}
async function task(fn){document.querySelectorAll('button').forEach(b=>b.disabled=true);status.textContent='Connecting…';try{await fn()}catch(e){status.textContent=e.message}finally{document.querySelectorAll('button').forEach(b=>b.disabled=false)}}
el('switch').href=(signup?'/sign-in':'/sign-up')+(pending?'?request='+encodeURIComponent(pending):'');
if(!pending){task(async()=>{const verifier=b64(crypto.getRandomValues(new Uint8Array(32))),challenge=b64(new Uint8Array(await crypto.subtle.digest('SHA-256',new TextEncoder().encode(verifier))));const a=await api('/v1/auth/start',{intent:signup?'sign-up':'sign-in',challenge,device:crypto.randomUUID(),redirect:'web'});sessionStorage.setItem('sj_pending',JSON.stringify({id:a.id,verifier}));location.replace(a.url)})}
el('email-form').onsubmit=e=>{e.preventDefault();task(async()=>{const r=await api('/v1/auth/email',{request:pending,email:el('email').value,intent:signup?'sign-up':'sign-in'});el('email-form').hidden=true;el('code-form').hidden=false;document.querySelector('footer').hidden=true;el('heading').textContent='Check your email';el('intro').textContent=signup?'Enter the six-digit code to create your sync account.':'Enter the six-digit code to sign in.';status.textContent=r.message;el('code').focus()})};
el('code-form').onsubmit=e=>{e.preventDefault();task(async()=>{const r=await api('/v1/auth/verify',{request:pending,code:el('code').value.trim()});el('code').value='';status.textContent='Signed in. Return to SonoJot to finish setting up this computer.';document.querySelectorAll('form,footer,.back').forEach(e=>e.hidden=true);const a=el('return');a.href=r.url;a.hidden=false;location.assign(r.url)})};`;

export function signinPage(request: Request): Response | null {
  if (request.method !== "GET") return null;
  const path = new URL(request.url).pathname;
  if (
    !["/", "/sign-up", "/sign-in", "/signin.css", "/signin.js"].includes(path)
  )
    return null;
  if (path === "/")
    return new Response(null, {
      status: 303,
      headers: { Location: "/sign-up" },
    });
  const signup = path === "/sign-up";
  if (new URL(request.url).searchParams.getAll("request").length > 1)
    throw new Fault(400, "Start sign-in again.");
  const title = signup ? "Create your sync account" : "Sign in";
  const html = `<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><meta name="robots" content="noindex"><title>${title} · SonoJot</title><link rel="stylesheet" href="/signin.css"><script src="/signin.js" defer></script></head><body><main><img class="brand" src="/assets/sonojot.svg" alt="SonoJot" width="160"><h1 id="heading">${title}</h1><p id="intro">${signup ? "Keep your words in sync. Start with your email; SonoJot will create your encryption key on your computer." : "Use the email for your sync account. Then approve this computer from one that is already set up."}</p><form id="email-form"><label>Email address<input id="email" name="email" type="email" autocomplete="email" maxlength="254" required autofocus></label><button>Continue</button></form><form id="code-form" hidden><label>Six-digit code<input id="code" name="code" inputmode="numeric" autocomplete="one-time-code" pattern="[0-9]{6}" minlength="6" maxlength="6" required></label><button>${signup ? "Create your sync account" : "Sign in securely"}</button></form><p id="status" role="status" aria-live="polite"></p><a id="return" hidden>Return to SonoJot</a><a class="back" href="${signup ? "/sign-up" : "/sign-in"}">Start again</a><footer><p>${signup ? "Already set up sync?" : "New to sync?"} <a id="switch" href="${signup ? "/sign-in" : "/sign-up"}">${signup ? "Sign in" : "Create your sync account"}</a></p></footer><noscript><p>Enable JavaScript to sign in securely.</p></noscript></main></body></html>`;
  return new Response(
    path === "/signin.css" ? css : path === "/signin.js" ? js : html,
    {
      headers: {
        "Content-Type": path.endsWith(".css")
          ? "text/css; charset=utf-8"
          : path.endsWith(".js")
            ? "text/javascript; charset=utf-8"
            : "text/html; charset=utf-8",
        "Content-Security-Policy":
          "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self'; font-src 'self'; connect-src 'self'; form-action 'self'; base-uri 'none'; frame-ancestors 'none'",
        "X-Robots-Tag": "noindex, nofollow",
      },
    },
  );
}
