import { Fault } from "./common";

// SonoJot's own emails. One calm layout, always sent as both HTML and plain text.
// Email-safe on purpose: tables, inline styles, no web fonts, no tracking and no
// links except sonojot.com and the support address.

export const SUPPORT = "support@sonojot.com";
export const SIGNIN_FROM = { name: "SonoJot", email: "signin@sonojot.com" };
export const NOTICE_FROM = { name: "SonoJot", email: "notices@sonojot.com" };

export type Mail = {
  from: { name: string; email: string };
  to: string;
  replyTo: string;
  subject: string;
  html: string;
  text: string;
};
export type MailSender = (env: Env, message: Mail) => Promise<void>;

export const sendMail: MailSender = async (env, message) => {
  if (env.MAIL_ENABLED !== "true")
    throw new Fault(503, "Email is not configured yet.");
  await env.MAIL.send(message);
};

/** The sign-in origin that serves the logo, only when it is on sonojot.com. */
export function mailOrigin(env: Pick<Env, "AUTH_ORIGIN">): string | null {
  const origin = env.AUTH_ORIGIN ?? "";
  return /^https:\/\/([a-z0-9-]+\.)*sonojot\.com$/.test(origin)
    ? origin
    : null;
}

const platforms: Record<string, string> = {
  windows: "Windows PC",
  macos: "Mac",
  linux: "Linux computer",
  ios: "iPhone",
};
/** The same plain device names the app uses. */
export function platformName(platform: string | null | undefined): string {
  return platforms[(platform ?? "").toLowerCase()] ?? "A new computer";
}

const months = [
  "January",
  "February",
  "March",
  "April",
  "May",
  "June",
  "July",
  "August",
  "September",
  "October",
  "November",
  "December",
];
/** For example "14 November 2026 at 15:42 UTC". */
export function when(seconds: number): string {
  const d = new Date(seconds * 1000);
  const two = (n: number) => String(n).padStart(2, "0");
  return `${d.getUTCDate()} ${months[d.getUTCMonth()]} ${d.getUTCFullYear()} at ${two(d.getUTCHours())}:${two(d.getUTCMinutes())} UTC`;
}

type Block = { text: string; kind?: "code" | "quiet" };
type Letter = {
  subject: string;
  preheader: string;
  heading: string;
  blocks: Block[];
  reason: string;
};

const ink = "#244437";
const muted = "#526358";
const page = "#e9e6dd";
const card = "#f3f0e7";
const line = "#d4d6cd";
const serif =
  "'Source Serif 4','Source Serif Pro',Georgia,'Times New Roman',serif";
// Georgia has old-style figures; the code needs even, lining digits.
const digits =
  "'Source Serif 4','Source Serif Pro','Palatino Linotype',Palatino,'Book Antiqua','Times New Roman',serif";

const escape = (s: string) =>
  s.replace(
    /[&<>"']/g,
    (c) =>
      ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[
        c
      ]!,
  );

function logo(origin: string | null): string {
  if (!origin)
    return `<p class="sj-ink" style="margin:0 0 28px;font-family:${serif};font-size:21px;line-height:1.2;font-weight:600;color:${ink};">SonoJot</p>`;
  const img = (variant: string) =>
    `<img src="${escape(origin)}/assets/sonojot-mail-${variant}.png" width="144" height="39" alt="SonoJot" style="display:block;border:0;outline:none;width:144px;height:39px;margin:0 0 28px;font-family:${serif};font-size:21px;font-weight:600;color:${ink};">`;
  // The light logo sits on an opaque paper background, so it stays readable when
  // a client darkens the email itself. Clients that honour the dark scheme
  // below swap in the cream logo on the dark card.
  return (
    img("light").replace("<img ", '<img class="sj-light-logo" ') +
    `<!--[if !mso]><!--><div class="sj-dark-logo" style="display:none;max-height:0;overflow:hidden;mso-hide:all;">${img("dark")}</div><!--<![endif]-->`
  );
}

function block(b: Block): string {
  if (b.kind === "code")
    return `<table role="presentation" cellpadding="0" cellspacing="0" border="0" style="margin:0 0 22px;border-collapse:separate;"><tr><td class="sj-well sj-ink" style="background:${page};border:1px solid ${line};border-radius:8px;padding:12px 10px 12px 20px;font-family:${digits};font-size:34px;line-height:1.2;font-weight:600;letter-spacing:0.3em;color:${ink};">${escape(b.text)}</td></tr></table>`;
  return b.kind === "quiet"
    ? `<p class="sj-muted" style="margin:0 0 16px;font-family:${serif};font-size:15px;line-height:1.5;color:${muted};">${escape(b.text)}</p>`
    : `<p class="sj-ink" style="margin:0 0 16px;font-family:${serif};font-size:17px;line-height:1.5;color:${ink};">${escape(b.text)}</p>`;
}

function render(
  letter: Letter,
  origin: string | null,
): { html: string; text: string } {
  const html = `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<meta name="x-apple-disable-message-reformatting">
<meta name="color-scheme" content="light dark">
<meta name="supported-color-schemes" content="light dark">
<title>${escape(letter.subject)}</title>
<style>
:root{color-scheme:light dark;supported-color-schemes:light dark}
body{margin:0;padding:0;-webkit-text-size-adjust:100%}
@media (prefers-color-scheme:dark){
.sj-page{background:#242824!important}
.sj-card{background:#3b443a!important;border-color:#4b5549!important}
.sj-well{background:#2c332b!important;border-color:#4b5549!important}
.sj-ink{color:#f3eedf!important}
.sj-muted,.sj-muted a{color:#c6cbbe!important}
.sj-light-logo{display:none!important}
.sj-dark-logo{display:block!important;max-height:none!important;overflow:visible!important}
}
</style>
</head>
<body class="sj-page" style="margin:0;padding:0;background:${page};">
<div style="display:none;max-height:0;overflow:hidden;mso-hide:all;">${escape(letter.preheader)}</div>
<table role="presentation" width="100%" cellpadding="0" cellspacing="0" border="0" class="sj-page" style="background:${page};">
<tr><td align="center" style="padding:32px 16px;">
<!--[if mso]><table role="presentation" width="480" cellpadding="0" cellspacing="0" border="0"><tr><td><![endif]-->
<table role="presentation" width="100%" cellpadding="0" cellspacing="0" border="0" style="max-width:480px;border-collapse:separate;">
<tr><td class="sj-card" style="background:${card};border:1px solid ${line};border-radius:14px;padding:36px 32px 20px;text-align:left;">
${logo(origin)}
<h1 class="sj-ink" style="margin:0 0 18px;font-family:${serif};font-size:25px;line-height:1.2;font-weight:600;letter-spacing:-0.01em;color:${ink};">${escape(letter.heading)}</h1>
${letter.blocks.map(block).join("\n")}
</td></tr>
<tr><td class="sj-muted" style="padding:18px 12px 0;text-align:center;font-family:${serif};font-size:13px;line-height:1.5;color:${muted};">
<p style="margin:0 0 6px;">${escape(letter.reason)}</p>
<p style="margin:0;">Questions? Reply to this email. &middot; <a href="https://sonojot.com" style="color:${muted};text-decoration:underline;">sonojot.com</a></p>
</td></tr>
</table>
<!--[if mso]></td></tr></table><![endif]-->
</td></tr>
</table>
</body>
</html>
`;
  const text = [
    "SonoJot",
    "",
    letter.heading,
    "",
    ...letter.blocks.flatMap((b) => [b.text, ""]),
    "--",
    letter.reason,
    "Questions? Reply to this email.",
    "https://sonojot.com",
    "",
  ].join("\n");
  return { html, text };
}

function mail(
  from: Mail["from"],
  to: string,
  letter: Letter,
  origin: string | null,
): Mail {
  return {
    from,
    to,
    replyTo: SUPPORT,
    subject: letter.subject,
    ...render(letter, origin),
  };
}

export type CodeMail = {
  to: string;
  code: string;
  /** Milliseconds since the epoch, from the provider's own expiry. */
  expiresAt: number;
  intent: "sign-up" | "sign-in";
  now?: number;
};
export function codeMail(m: CodeMail, origin: string | null): Mail {
  if (!/^\d{6}$/.test(m.code)) throw Error("invalid_code");
  const minutes = Math.max(
    1,
    Math.round((m.expiresAt - (m.now ?? Date.now())) / 60000),
  );
  const expires = `It expires in ${minutes} ${minutes === 1 ? "minute" : "minutes"}.`;
  const signup = m.intent === "sign-up";
  return mail(
    SIGNIN_FROM,
    m.to,
    {
      subject: `${m.code} is your SonoJot code`,
      preheader: expires,
      heading: signup ? "Create your sync account" : "Your sign-in code",
      blocks: [
        { text: m.code, kind: "code" },
        {
          text: signup
            ? `Enter it on the SonoJot page to create your sync account. ${expires}`
            : `Enter it on the SonoJot sign-in page. ${expires}`,
        },
        { text: "If you didn't ask for this, you can ignore it.", kind: "quiet" },
      ],
      reason: signup
        ? "Sent because this address was entered to create a SonoJot sync account."
        : "Sent because this address was entered on the SonoJot sign-in page.",
    },
    origin,
  );
}

export type NoticeKind =
  | "new-device"
  | "identity-updated"
  | "email-changed"
  | "reset-requested";
export const noticeKinds: NoticeKind[] = [
  "new-device",
  "identity-updated",
  "email-changed",
  "reset-requested",
];
export type Notice = {
  to: string;
  /** Seconds since the epoch when the event happened. */
  at: number;
  detail?: string | null;
};
const nothing = "If this was you, there's nothing to do.";
export function noticeMail(
  kind: string,
  n: Notice,
  origin: string | null,
): Mail {
  const at = when(n.at);
  let letter: Letter;
  if (kind === "new-device") {
    const name = platformName(n.detail);
    letter = {
      subject: `${name} joined your SonoJot sync`,
      preheader: nothing,
      heading: `${name} joined sync`,
      blocks: [
        {
          text: `${name} signed in to your sync account on ${at}. Your other computers hand it your key on their own.`,
        },
        { text: nothing },
        {
          text: "If it wasn't, open SonoJot on one of your computers, go to Account and sync, and choose Not me. That removes it and changes your key.",
        },
      ],
      reason:
        "Sent to the email for your SonoJot sync account whenever a computer joins.",
    };
  } else if (kind === "identity-updated") {
    letter = {
      subject: "Your SonoJot sign-in email changed",
      preheader: "Security notices keep coming to this address for now.",
      heading: "Your sign-in email changed",
      blocks: [
        {
          text: `The email you use to sign in to SonoJot sync was changed on ${at}. Security notices keep coming to this address until you confirm the change.`,
        },
        {
          text: "If this was you, open SonoJot on one of your computers, go to Account and sync, and choose Confirm contact email.",
        },
        {
          text: "If it wasn't, reply to this email right away so we can help you secure your sign-in.",
        },
      ],
      reason: "Sent to the contact email for your SonoJot sync account.",
    };
  } else if (kind === "email-changed") {
    letter = {
      subject: "Your SonoJot contact email changed",
      preheader: "This notice goes to your previous address.",
      heading: "Your contact email changed",
      blocks: [
        {
          text: `The contact email for your SonoJot sync account was changed on ${at}. This notice goes to your previous address.`,
        },
        { text: nothing },
        {
          text: "If it wasn't, open SonoJot on one of your computers and go to Account and sync. Remove any computer you don't know, which changes your key, then secure your email accounts.",
        },
      ],
      reason: "Sent to the previous contact email for your SonoJot sync account.",
    };
  } else if (kind === "reset-requested") {
    letter = {
      subject: "Someone asked to reset your SonoJot sync",
      preheader: "Opening SonoJot on one of your computers cancels it.",
      heading: "Sync reset requested",
      blocks: [
        {
          text: `Someone asked to reset your SonoJot sync on ${at}. The reset can't finish for 72 hours, and the request expires 7 days after that.`,
        },
        { text: nothing },
        {
          text: "If it wasn't, open SonoJot on one of your computers. That cancels the reset on its own.",
        },
      ],
      reason: "Sent to the contact email for your SonoJot sync account.",
    };
  } else throw Error("unknown_notice");
  return mail(NOTICE_FROM, n.to, letter, origin);
}
