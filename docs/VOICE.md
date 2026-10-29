# Voice Mode

A hands-free call with every thread in Keplorayx. Architecture:
[ARCHITECTURE.md › Voice](../ARCHITECTURE.md#voice-dictation-and-voice-mode). Code:
`crates/dove-voice` (no UI) and `apps/dove/src/voice_call.rs` (the app side).

## Dictation

Hold the mic in the composer (or click to toggle). With a local speech model
installed (Settings › Voice), **live dictation** is on: words appear in the
composer while you talk, the phrase you're saying updates in place, and each
pause (about 0.65 s) commits it. Text is inserted, never sent. Stopping
commits the last phrase; Escape cancels the phrase in progress. Live
dictation uses the Voice Mode model (Moonshine base unless Parakeet is
chosen) and shares its loaded copy with calls. Turn it off in Settings ›
Voice › Live dictation to get the older behaviour (Parakeet transcribes after
you stop).

## Using it

1. Settings › Voice › Voice Mode models: download Moonshine (66 MB), Silero VAD
   (2.2 MB) and Smart Turn (8.7 MB). Or use Parakeet from Dictation instead of
   Moonshine (more accurate, about 700 MB of memory while a call runs).
2. Click the phone button beside the mic, or press Ctrl/Cmd+Shift+Space
   (Settings › Shortcuts can change it). On Windows the shortcut also works
   while another app is in front (Settings › Voice › Shortcut outside Keplorayx;
   if another app already owns the keys, Settings says so). Text already in
   the composer is sent first.
3. Talk. The call strip sits on the input you are talking into (a thread's,
   or the new-thread composer on Home; on other pages it floats at the foot
   of the page). It shows Listening / Hearing you / Working on it / Speaking,
   who the call talks to, a live waveform and the call's length, the live
   caption or the reply being read, and buttons to mute the mic, stop Keplorayx
   talking (the agent keeps working) and end the call. The phone button stays
   lit while a call runs. A soft chime marks when the call starts listening
   and when Keplorayx has heard you (Settings › Voice › Sounds). Calls continue
   while you move around Keplorayx.

**No thread yet?** Start a call from Home and say what you want done. The
first request starts a new thread on the agent, model, effort, permission
mode and project picked in the composer, exactly as typing it and pressing
Send would, and the call then talks to that thread.

| Say | Does |
| --- | --- |
| "What's happening?" / "Status" | Status of the current thread from Keplorayx's own state |
| "What's everyone doing?" | Every thread: counts, then the ones that need you |
| "Stop" | Interrupt the current thread and stop speaking |
| "Approve" / "Deny" / "Approve for this session" | Answer the waiting request (session scope only where the agent has it) |
| "Talk to the tests thread" / "Switch to Codex" | Change the thread the call talks to |
| "Take me back" | The previous thread |
| "Claude, how's the auth fix?" | Status of a named thread |
| "Tell Codex to also run clippy" | Steer (or send to) a named thread |
| "Start a Gemini task on the landing page in a new worktree" | A new thread, in a new worktree |
| "Launch three Claude chats to review the auth code" / "Open two Codex terminals" / "Spin up a terminal as the Builder" | Several threads at once, in chat or the agent's own terminal UI (up to 8). Keplorayx says what it will open and waits for "yes" (or the card's **Open them**); the first becomes the open thread and the rest open in panes beside it |
| "What's the team doing?" / "How's the Reviewer doing?" | What each teammate is working on, from their threads |
| "Ask the Reviewer to look at the login diff" / "Tell Builder the API changed" | Goes to the teammate's latest thread in the project (or a new one as them); their answer is read to you |
| "Draft for Codex: run the tests on Windows" / "Type cargo test into the codex terminal" | Puts the words in that thread's input (its composer, or the terminal's prompt line) without sending. Then "send it", "change it to …", "add …" or "never mind" (the call view shows the draft with Send and Clear) |
| "Open the tests thread beside this" / "Split with Codex" | Shows that thread in a pane next to the open one |
| "Show the diff" | Opens the Changes panel |
| "Take a look at this" / "Look at the page" / "Look at the diff" | Attaches what you see to your next message: the open browser page or diff, or (Windows) a capture of the last other app's window you were in |
| "Talk faster" / "slower" · "Mute" · "Be quiet" · "Repeat that" · "End call" | Call controls |
| Anything else | Sent to the current thread as your message (with no thread open, starts one on the picked agent and model) |

Push-to-talk (Settings › Voice › Listening): hold the orb, or hold Space when
the call is muted and no text field has focus.

**What you can say.** The call view's **What you can say** button lists
everything above, grouped, with a phrase to try for each. When the call
waits on an answer (several threads to open, a draft to send), the strip
over the composer and the call view both show it with two buttons, so you
can click instead of saying "yes" or "no".

**Talking to one thread among many.** The call shortcut (Ctrl/Cmd+Shift+Space)
starts a call on the thread you're in, including a thread in a pane beside
the open one or a terminal. While a call runs, pressing it in another thread
moves the call there ("Talking to Codex on …") instead of ending it; press it
again in the same thread to hang up. Opening a thread moves the call too, and
in the call view a click on any thread in the list talks to it.

## Safety

- Voice answers the same permission request a click answers. It never changes a
  thread's permission mode and never answers an agent's question for you.
- Destructive requests are never approved by voice alone: recursive deletes,
  force pushes, history rewrites, database drops, `curl | sh`, secrets
  (`.env`, keys, `.ssh`, `.aws`), system permission changes, and paths outside
  the thread's folder. Keplorayx says why, asks you to say a confirmation phrase
  ("confirm delete"), and then waits for your click on Approve in the chat. If
  the request changes before you click, the confirmation is dropped.
- A command router, when turned on, only proposes; every intent is checked
  against a strict schema and Keplorayx's current state before anything runs.
  It can be an installed agent (a tool-less, read-only run in an empty
  scratch folder on its cheapest model) or an OpenAI-compatible endpoint.
- Keplorayx never reports progress it has not seen: statuses come from the
  thread's events, todos, current activity and `git diff --shortstat`.

## Privacy

Everything runs on this computer by default. Audio stays in memory for the
current turn and is never written to disk. `~/.dove/voice/calls.jsonl` records
each call's text turns (what you said, what Keplorayx said, which thread) so a call
can be reviewed; delete it any time.

Optional cloud features are off until you turn them on, and Settings says what
each sends:
- **Cloud speech recognition** uploads your spoken audio: to an
  OpenAI-compatible endpoint (`DOVE_VOICE_API_KEY`) or Deepgram
  (`DEEPGRAM_API_KEY`, model `nova-3` unless set).
- **Cloud voice** uploads what Keplorayx says, which includes agents' replies, to
  the service you pick: Fish Audio (`FISH_AUDIO_API_KEY`; the free
  `s2.1-pro-free` model by default), Deepgram Aura-2 (`DEEPGRAM_API_KEY`),
  Cartesia Sonic (`CARTESIA_API_KEY`), Inworld (`INWORLD_API_KEY`),
  ElevenLabs (`ELEVENLABS_API_KEY`, Flash v2.5) or an OpenAI-compatible
  endpoint. All but the last stream 16 kHz PCM, so speech starts on the first
  chunk. If the service fails, Keplorayx says so once and uses the system voice.
  Prices, free tiers and startup programs: [VOICE_PROVIDERS.md](VOICE_PROVIDERS.md).
- **Command router** sends the request and your threads' titles to the agent
  or endpoint you chose.
- **Look at this window** writes a PNG to `~/.dove/voice/look/` and gives the
  agent its path; the agent's own provider sees it as it would any attachment.
Keys are read from the environment, never stored in Keplorayx's database.

## Models

| Model | Use | Size | Licence | Source (pinned revision, SHA-256 checked) |
| --- | --- | --- | --- | --- |
| Moonshine base, int8 | Speech recognition (default) | 66 MB | MIT | `onnx-community/moonshine-base-ONNX` `b1e9b6a` |
| Moonshine tiny, int8 | Not offered in the UI (see below) | 32 MB | MIT | `onnx-community/moonshine-tiny-ONNX` `a6da124` |
| Parakeet TDT 0.6B v3, int8 | Speech recognition (accurate option; dictation) | 670 MB | CC-BY-4.0 | `istupakov/parakeet-tdt-0.6b-v3-onnx` `8f23f0c` |
| Silero VAD v5 | Voice activity | 2.2 MB | MIT | `onnx-community/silero-vad` `e71cae9` |
| Smart Turn v3.2 (CPU) | End of turn | 8.7 MB | BSD-2-Clause | `pipecat-ai/smart-turn-v3` `f766f81` |
| System voice | Speech | 0 | OS | Windows speech synthesis, macOS `say`, Linux `espeak-ng` |
| Cloud voices (optional) | Speech | 0 | Provider terms | Fish Audio, Deepgram, Cartesia, Inworld, ElevenLabs; see [VOICE_PROVIDERS.md](VOICE_PROVIDERS.md) |

Choices made from measurements on this project's test machine:
- **Moonshine base over Parakeet by default:** 162 MB against 668 MB resident,
  1.4–1.8 s against 5.6 s to load, similar per-turn compute (291 ms against
  290 ms p50) and the same transcripts on the fixtures.
- **Moonshine tiny not offered:** 190 ms against 334 ms p50, but it returned an
  empty transcript for "Stop.", the command that matters most.
- **Kokoro, Pocket TTS and Supertonic were not adopted for speech.** Kokoro
  needs a phonemizer (espeak-ng) and ~80 MB+ of weights; Supertonic's weights
  are OpenRAIL-M with use restrictions. The system voice renders a first
  clause in 13–21 ms with no download and no memory cost. A neural voice can
  be added behind the `TextToSpeech` trait.
- **Echo cancellation is a Keplorayx NLMS filter, not WebRTC AEC3**, to avoid a C++
  build dependency. It removes more than 20 dB of a synthetic echo in tests but
  is weaker than AEC3 on loud, non-linear laptop speakers, so barge-in also
  needs 128 ms of strong speech while Keplorayx talks.

## Benchmark

`cargo run -p dove-voice --release --example voice_bench -- --root <models dir> --parakeet ~/.dove/voice/parakeet-v3`

**Machine:** Intel Core i7-10810U (6 cores / 12 threads, 1.1 GHz base),
15.8 GB RAM, Windows 11 Pro 10.0.26200, release build, default speaker,
ONNX Runtime 1.28 via `ort` 2.0.0-rc.13 (shared by every model). Run on
2026-10-22.

**Method.** Fixture speech is rendered by the Windows voice from four phrases
("What is happening with the tests thread?", "Tell Codex to also run clippy on
Windows.", "Add a retry with exponential backoff to the token refresh.",
"Stop."), resampled to 16 kHz and fed through the real `Listener` (Silero,
NLMS, Smart Turn) with 600 ms of quiet before and 2.5 s after. End-to-end
figures add, per turn, the audio time the detector waits for (a pause is real
time) to the compute measured in the run; stage averages are never summed.
Playback timings use the real speaker. The microphone is not opened unless
`--live-mic` is passed, so warm start excludes opening it.

| Metric | Target | Measured (final run) | Verdict |
| --- | --- | --- | --- |
| Cold call startup to listening-ready | < 1500 ms | 1753 ms (1364 ms in an earlier run of the same code) | Not reliably met |
| Warm call startup (engines cached) | < 100 ms | < 1 ms, excluding opening the microphone | Pass (mic open not measured) |
| Active local stack RAM | < 400 MB | 162 MB | Pass |
| Final transcript after the turn decision | < 120 ms | p95 250 ms | Fail |
| Text ready after the last word (includes the pause) | – | p50 417 ms, p95 1082 ms | – |
| End-of-turn decision | < 60 ms | p95 61.4 ms (53.1–58.6 ms in earlier runs) | Borderline |
| Fast-path command to first audio | < 250 ms | p50 21 ms, max 48 ms | Pass |
| Added overhead on an LLM turn (excl. TTFT) | < 300 ms | 446 ms (p50 path) | Fail |
| Barge-in playback stop | < 100 ms | p50 2.4 ms, max 8.7 ms (voice onset to silence ≈ 130 ms) | Pass |
| Listening CPU while silent | < 3% of a core | 0.94% (0.31% earlier) | Pass |
| Idle memory when Voice Mode is unused | 0 MB | 0: nothing loads until a call starts | Pass |
| Memory after a call's engines are dropped | 0 MB | +30 MB | Fail |
| Recognition compute per turn | – | Moonshine p50 291 / p95 404 ms; Parakeet p50 290 ms | – |
| Per-frame listening cost (VAD + AEC) | – | p50 0.16 ms, p95 0.29 ms per 32 ms frame | – |

**Where the misses come from.**
- *Transcript after the decision (250 ms):* Moonshine base spends about 60 ms
  per second of audio in its encoder and 17–18 ms per decoded token on this
  CPU. Speculation starts recognition 128 ms into a pause, which hides part of
  it. The split (non-merged) decoder export was measured (same int8
  weights, same machine, 15 steps): 34.8 ms against 38.4 ms per step in
  ONNX Runtime from Python, about 10% faster, but it loads a second copy of
  the decoder weights (about 80 MB more resident), so it was not adopted.
  Next step: an encoder pass over the audio while the user is still talking.
- *LLM-turn overhead (446 ms):* mostly the pause before a turn ends (p50 224 ms
  of audio) plus the transcript. A shorter pause cuts people off; Smart Turn
  lets it be short when the sentence sounds finished.
- *Cold start:* ONNX Runtime building the Moonshine sessions (1.4–1.8 s even
  with the encoder and decoder built in parallel and hashes checked
  alongside). Later calls within five minutes start warm.
- *Memory after unload (+30 MB):* allocator and ONNX Runtime pages kept after
  the sessions are dropped. A Keplorayx that never starts a call loads none of it.
- *Turn decision (53–61 ms):* Smart Turn's 8 s window (17 ms of it is the
  log-mel front end, the rest the model on 4 threads).

`dove-voice` also measured: real-thread end-to-end (`tests/call.rs`, pushed
audio, Moonshine): ready in 1.27 s; "What is happening with the test thread?"
and "Stop" transcribed in order; with the synthetic voice, Smart Turn judged
the first sentence unfinished, so it ended on the 1 s pause fallback.

Not measured here: macOS and Linux (no machines), real microphones and
human voices at scale, accuracy beyond the four fixtures, and the live
microphone's CPU and start-up time (`--live-mic` exists but was not run, so as
not to open this machine's microphone unattended).

### Live dictation

`cargo run -p dove-voice --release --example dictation_bench -- --root <models dir>`
renders three sentences with the Windows voice and streams them in real time
(1.2 s pauses) into live dictation with Moonshine base. Same machine,
2026-10-22:

| Phrase | First live text after the phrase starts | Committed after the clip ends |
| --- | --- | --- |
| 1 | 866 ms | 408 ms |
| 2 | 741 ms | 440 ms |
| 3 | 828 ms | 305 ms |

Live text refreshed about every 0.5 s (22 updates for three sentences); the
model was ready 1.45 s after a cold start. "Committed" is measured from the
end of each rendered clip, which includes the synthetic voice's trailing
silence, so the gap after the last word is longer by that amount (the commit
waits for a 0.65 s pause). All three sentences were transcribed correctly
except "pull request" (live) committing as "poll request": the final pass on
the trimmed phrase can differ slightly from the last live text.

## Testing without a microphone

`DOVE_VOICE_INPUT=silence` (or a path to a WAV file) feeds a call in place of
the microphone, which is then never opened; the file starts
`DOVE_VOICE_INPUT_DELAY_MS` (default 8000) after the call does, since audio
from before the models are ready is dropped. `DOVE_VOICE_OUTPUT=silent` keeps
replies off the speaker. With both, a scratch `DOVE_HOME` and the models in
`<DOVE_HOME>/voice/call`, a spoken request recorded with the system voice
runs the whole path: recognition, the conductor, a new thread on the
picker's model, and the reply on the strip.

## Troubleshooting

- **"Voice Mode needs local speech recognition."** Download Moonshine (or
  Parakeet) in Settings › Voice.
- **Keplorayx hears itself on speakers.** Keep Echo cancellation on; lower the
  volume or use headphones; push-to-talk avoids the problem entirely.
- **It cuts me off mid-sentence.** Install Smart Turn; without it Keplorayx ends a
  turn after 550 ms of silence (1 s after words like "and" or "the").
- **"Speaker unavailable."** The call keeps listening and shows replies in the
  chat; check the default output device.
- **The microphone disconnected.** The call ends; threads are unaffected.
  Start a new call.
- **No system voice on Linux.** Install `espeak-ng`.
