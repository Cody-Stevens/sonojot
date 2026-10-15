# SonoJot Privacy Notice

Version: 1 · Effective October 13, 2026

## Summary

SonoJot is a free, open-source dictation and meeting-transcription app for Windows, macOS, and Linux made by Cody Stevens Studio LLC (the "Studio", "we", "us"). It is built to work on your computer, and its source code is public under the Apache License 2.0, so you can check what it does.

- **By default, your speech, recordings, transcripts, and text stay on your computer.** Speech recognition, cleanup, punctuation, and meeting notes run locally.
- **SonoJot collects no data on its own.** It has no user accounts, analytics, or telemetry. It sends a crash or bug report only when you choose to send one (section 3). The Studio does not receive your recordings, dictations, or transcripts. Update and download requests carry only ordinary network information, such as your IP address, as described in sections 3 and 4.
- **SonoJot connects to the internet** to check for updates, to download models and tools you choose, and, only if you connect it, to send content to OpenRouter for online processing.

| Stays on your computer | Can leave your computer |
|---|---|
| Dictation history (text), meeting recordings (audio), transcripts, notes, titles, vocabulary, learned corrections, settings, diagnostics, crash dumps | Update checks and update downloads (to the Studio's update server on Cloudflare; not on Linux) |
| Your OpenRouter key, protected by Windows, macOS, or your Linux keyring | Model, runtime, and media-converter downloads (from Hugging Face, GitHub, Moonshine, PyPI, and an FFmpeg build host) |
| | Only if you connect OpenRouter: meeting transcripts, dictation excerpts, or meeting audio for the online features you start |

## 1. Who we are and what this covers

Cody Stevens Studio LLC, a Texas limited liability company, makes SonoJot. This notice covers the SonoJot desktop app and the two web resources the app uses on sonojot.com: the OpenRouter sign-in relay page and the model-recommendations file. It also covers visits to the sonojot.com website, which sets no cookies, has no advertising, and counts visits with Cloudflare Web Analytics as described in section 4.

## 2. What SonoJot stores on your computer

SonoJot stores the following in its data folder: `%LOCALAPPDATA%\SonoJot` on Windows, `~/Library/Application Support/SonoJot` on macOS, or `~/.local/share/SonoJot` on Linux. Use **About → Troubleshooting → Open data folder** to see it.

- **Dictation history.** The text of each dictation, including the raw recognized text and the cleaned text. SonoJot does not keep dictation audio. History is saved by default. Turn on "do not save dictations" to stop saving it, or delete single entries or all of them.
- **Meeting recordings.** Audio files of your microphone, your computer's audio output, and a mixed track, plus any recordings you import. Meetings record both your microphone and your computer's audio by default, so recordings contain other people's voices.
- **Meeting transcripts, notes, titles, and speaker labels.**
- **Vocabulary and learned corrections.** Words you add, and corrections SonoJot learns from your edits if you turn on learning (off by default). Learned entries are removed when the dictations they came from are deleted.
- **Settings**, including your model choices and your OpenRouter model choices.
- **Your OpenRouter key**, if you connect OpenRouter. On Windows it is encrypted with Windows Data Protection for your user account. On macOS it is stored in your login keychain. On Linux it is stored in your desktop keyring through the Secret Service (for example GNOME Keyring), and the data folder keeps only a reference to it. Disconnecting removes it.
- **Downloaded models, runtimes, and license files.**
- **Diagnostics.** A small, size-limited log of technical events, such as timings and error categories. It is designed not to contain your text or file paths.
- **Crash dumps (Windows).** If a speech-recognition worker crashes, SonoJot may save a crash dump that can contain stack and module information, and occasionally fragments of memory. It keeps at most two, for up to seven days, and saves none while "do not save dictations" is on.

SonoJot itself does not encrypt these files, except your OpenRouter key. Anyone with access to your computer account may be able to read them. Protect your account, and consider full-disk encryption such as BitLocker or FileVault.

When SonoJot pastes dictated text, it uses your system clipboard. If you use clipboard history or clipboard sync, your operating system may store or sync that text.

## 3. What leaves your computer, and when

### Update checks and updates

About 30 seconds after it starts, then about every six hours, and when you open or return to its window if it has not checked in the past hour, SonoJot requests a small file from `releases.sonojot.com`, which the Studio hosts on Cloudflare R2. The request includes your IP address and standard request information, but none of your content. If an update is available, SonoJot tells you. It downloads the update from the same host only when you choose, and installs it only when you restart to update. SonoJot currently has no setting to turn off update checks. On Linux, SonoJot makes no update checks; you update it by installing a newer package.

### Model, runtime, and media-converter downloads

When you choose a model in setup or in the model library, SonoJot downloads it and any runtime it needs directly from the publisher's host: Hugging Face (`huggingface.co` and its download servers), GitHub, Moonshine (`download.moonshine.ai`), or PyPI (`files.pythonhosted.org`) for NVIDIA GPU components.

SonoJot can also download the FFmpeg media converter, used to import recordings: from GitHub on Windows, or from `releases.sonojot.com` on macOS, where the Studio hosts an LGPL build together with its source code. On Linux, SonoJot does not download FFmpeg; it uses one you install. Before downloading FFmpeg or NVIDIA GPU runtime components, SonoJot shows their license terms, during setup or when you choose to install them.

These hosts receive your IP address, request details, and a user agent that identifies SonoJot and its version. Their own privacy policies apply. On Windows, the installer may also download Microsoft's WebView2 runtime if your computer does not have it.

### OpenRouter (optional, off by default)

SonoJot sends nothing to OpenRouter unless you connect your own OpenRouter account. After you connect:

- **Signing in.** Your browser opens OpenRouter's sign-in page. OpenRouter returns a one-time code to `sonojot.com/connect/openrouter`, which passes it straight to SonoJot on your computer. SonoJot then exchanges it for a key. The relay page never sees your key.
- **Choosing a model.** SonoJot downloads the Studio's list of recommended models from `sonojot.com` and OpenRouter's public model catalog. These requests contain none of your content.
- **Meeting notes.** When you process notes with an online model, SonoJot sends the meeting transcript, in sections, together with the notes instructions.
- **Vocabulary review.** When you run a vocabulary review with online processing selected, SonoJot sends excerpts of the recognized and cleaned text of up to your 20 most recent saved dictations.
- **Online meeting transcription.** When you choose OpenRouter's speech model to transcribe a meeting, SonoJot sends the meeting audio in short chunks.

SonoJot never sends dictation audio to OpenRouter, and automatic meeting titles always run locally.

OpenRouter, Inc. receives this content and passes it to the model provider that serves your request. Your OpenRouter account's settings, OpenRouter's terms and privacy policy, and the provider's own policies govern how they handle it. SonoJot asks OpenRouter to use only providers that do not collect user data, which may make some models unavailable. The Studio cannot guarantee how OpenRouter or any provider handles your content. OpenRouter bills your account for usage. To stop, choose local processing and select Disconnect.

If your content includes other people's voices or words, make sure you have their consent before you send it online. Section 4 of the SonoJot Terms of Use explains your responsibilities.

### Crash and bug reports (only when you send one)

If SonoJot closed unexpectedly, it offers a crash report the next time it starts. You can also report a bug from the app. Nothing is sent unless you tick the box and choose Send, and you can see the whole report first. A report contains the notes you write, SonoJot's version, your operating system and processor type, and, if you include them, the crash location and stack trace, your setup and model choices, a hardware summary (memory, processor cores, graphics card), and recent entries from the diagnostics log. It never contains your recordings, dictations, transcripts, vocabulary, file paths, or OpenRouter key. SonoJot sends it through `sonojot.com` to the Studio's private issue tracker. Cloudflare sees your IP address as it delivers the request, and it is used only to limit how many reports one address can send; it is not stored with the report.

## 4. What the Studio receives

- **No content.** The Studio does not receive your recordings, dictations, transcripts, notes, vocabulary, settings, diagnostics, crash dumps, or OpenRouter key. The only exception is a crash or bug report you choose to send, which contains what section 3 describes.
- **No accounts, analytics, or telemetry.** SonoJot has no sign-in with the Studio and contains no analytics or advertising. It sends crash and bug reports only when you choose to.
- **Request data held by Cloudflare.** Cloudflare hosts `releases.sonojot.com` and `sonojot.com` and processes request information such as IP address, time, requested file, and user agent to deliver files and protect its network. The Studio sees only the aggregate traffic statistics Cloudflare provides for these hosts, such as request counts by country and file. The Studio does not keep request logs and does not use this information to identify individual users. Cloudflare's own privacy policy governs the request data it processes.
- **Website visit counts.** The sonojot.com website, but not the SonoJot app, uses Cloudflare Web Analytics. When a page loads, a small script reports the page address, the referring site, load times, and your browser, operating system, device type, and country. It sets no cookies, stores nothing on your device, and does not use your IP address to identify you or follow you across sites. The Studio sees only aggregate counts, such as page views by page and country.
- **Hugging Face statistics.** For models the Studio publishes on Hugging Face, such as the SonoJot punctuation models, Hugging Face may show the Studio aggregate download counts.
- **Messages you send us.** If you email us, we receive your email address and whatever you include, and we use them to reply. If you open an issue on GitHub, it is public, and GitHub's privacy policy applies to it.

The Studio does not sell or share personal information for advertising, and does not use any information from SonoJot to train AI models. If the Studio ever offers a way to contribute recordings or text to improve SonoJot's punctuation or cleanup models, it will be opt-in, will be announced separately, and will come with its own notice. No such program exists today.

## 5. Other people in your recordings

Meeting recordings and transcripts can contain other people's voices, names, and statements. That information is stored on your computer under your control, and you decide whether to keep, share, or send it online. The Studio has no copy and cannot access, correct, or delete it. If you were recorded by a SonoJot user, contact that person. You are responsible for obtaining any consent the law requires before recording; see section 4 of the Terms of Use. Before your first meeting recording, SonoJot shows a reminder about consent, but it does not notify other participants.

## 6. Children

People under 18 may use SonoJot with a parent's or guardian's permission. SonoJot has no accounts and sends the Studio no information, so the Studio does not collect personal information from children through the app. Online services you connect have their own age limits; OpenRouter requires users to be at least 18. If you believe a child has sent us personal information, for example by email or in a GitHub issue, contact support@sonojot.com and we will delete what we control.

## 7. Security

SonoJot verifies the checksum of every model and tool it downloads against a pinned value. It installs only updates signed with the Studio's update key, connects to online services over HTTPS, and runs its local model servers on your computer's loopback address only. It protects your OpenRouter key with your operating system's credential protection. No software is perfectly secure, and the files SonoJot keeps on your computer depend on the security of your computer and account.

## 8. How long information is kept

| Information | Where | How long |
|---|---|---|
| Dictations, meetings, transcripts, notes, vocabulary, settings | Your computer | Until you delete them or delete the data folder |
| Crash dumps | Your computer | At most two, up to seven days |
| Diagnostics log | Your computer | Size-limited; older entries are replaced |
| Update packages | Memory | Discarded if SonoJot quits without installing |
| Cloudflare request data | Cloudflare | Under Cloudflare's policies; the Studio sees aggregate statistics only |
| Website visit counts | Cloudflare | Under Cloudflare's policies; the Studio sees aggregate counts only |
| Content sent to OpenRouter | OpenRouter and model providers | Under their policies and your OpenRouter settings |
| Support email | Studio mailbox | As long as needed to help you, and no more than two years after the conversation ends |
| Crash and bug reports you send | The Studio's private issue tracker | As long as needed to investigate and fix the problem it describes |

Uninstalling SonoJot may leave the data folder on your computer. Delete the folder to remove your data.

## 9. Your choices

- Use SonoJot fully offline for dictation by not connecting OpenRouter. Online features stay off unless you connect and choose them.
- Disconnect OpenRouter at any time.
- Turn on "do not save dictations", or delete single dictations or all of them.
- Delete meetings, which removes their audio and transcripts.
- Record your microphone only, instead of your microphone and your computer's audio.
- Leave learning from corrections off (the default), or turn it off later.
- Block SonoJot's network access with your firewall. Updates, downloads, and online features will then stop working.

## 10. Your privacy rights

Depending on where you live, you may have rights to access, correct, or delete personal information about you, or to object to certain processing. SonoJot keeps your content on your computer, so you can exercise most of these rights directly in the app. For information the Studio holds, such as support emails or request data available to it, contact support@sonojot.com. We may need to verify your request and will respond within the time the law requires. Do not send passwords, keys, government identification, or recordings with a request.

The Studio may disclose information it holds if required by law, or to protect people's rights and safety. It holds no copy of your SonoJot content.

## 11. Where information is processed

The Studio is based in the United States. Cloudflare, OpenRouter, model providers, and download hosts may process request data and any content you send them in the United States and other countries.

## 12. Changes to this notice

Each version of this notice has a date. If we make a material change, we will post the new notice at sonojot.com/privacy with a new date, and the Terms version will change so SonoJot asks you to agree again.

## 13. Contact

- Email: support@sonojot.com
- Public issues (not for personal information): https://github.com/Cody-Stevens/sonojot/issues
- Legal notices: Cody Stevens Studio LLC, 5900 Balcones Drive #30714, Austin, Texas 78731

Cody Stevens Studio LLC, a Texas limited liability company.
