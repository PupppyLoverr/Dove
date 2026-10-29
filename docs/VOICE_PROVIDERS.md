# Voice providers: what to use, what it costs, what's free

Keplorayx's voice runs on this computer by default: speech recognition (Moonshine
or Parakeet) and dictation never need the network. The weak spot of a fully
local setup is the *speaking* voice: the operating system's voices sound
robotic next to neural ones. Keplorayx can speak through a cloud voice instead,
streamed so the first words play while the rest of the sentence is still
being made. This page compares the services Keplorayx supports, their prices and
the free or startup programs that cover them.

Research date: 2026-10-22. Prices change often; check the linked pages before
you budget. Figures marked *reported* come from third-party comparisons, not
the provider's own page.

## Recommendation

| Use | Pick | Why |
| --- | --- | --- |
| Free, natural voice now | **Fish Audio, `s2.1-pro-free`** | Fish Audio's S2.1 Pro is free for developers through the API; no card. Keplorayx's default model for Fish Audio. |
| Lowest latency, paid | **Deepgram Aura-2** | Plain REST that streams raw PCM; new accounts get $200 of credit. |
| Real-time agent voice, free to try | **Cartesia Sonic** | 20,000 free credits a month (non-commercial use only). |
| Quality per dollar | **Inworld** | Natural voices, streams NDJSON. |
| If you have a grant | **ElevenLabs Flash v2.5** | Strong voices; generous startup grant (below). |
| Fully offline | System voice | No download, no account; robotic. |

Dictation and Voice Mode recognition stay local either way (Moonshine base is
the quickest: about 160 MB in use, MIT licence).

## Text-to-speech services

| Service | Price | Free | First audio | Keplorayx setting |
| --- | --- | --- | --- | --- |
| [Fish Audio](https://fish.audio/developers/) | S2.1 Pro: $15 per 1M UTF-8 bytes | `s2.1-pro-free` model free for developers; credits to start, no card | Sub-second (provider's claim) | `fish`, key `FISH_AUDIO_API_KEY` (or `FISH_API_KEY`) |
| [Deepgram Aura-2](https://developers.deepgram.com/reference/text-to-speech/speak-request) | $0.030 per 1,000 characters (*reported*) | $200 credit on sign-up (*reported*) | ~90 ms (*reported*) | `deepgram`, `DEEPGRAM_API_KEY` |
| [Cartesia Sonic](https://docs.cartesia.ai/api-reference/tts/bytes) | Plans from $5/month for 100K credits (*reported*) | 20,000 credits/month, non-commercial (*reported*) | ~40 ms (*reported*) | `cartesia`, `CARTESIA_API_KEY` |
| [Inworld](https://docs.inworld.ai/api-reference/ttsAPI/texttospeech/synthesize-speech-stream) | TTS-2: $25 per 1M characters on demand, $12.50 Growth (*reported*) | ~70 minutes on pay-as-you-go (*reported*) | < 250 ms P90 (*reported*) | `inworld`, `INWORLD_API_KEY` |
| [ElevenLabs](https://elevenlabs.io/startup-grants) | Flash ~$40–50 per 1M characters (*reported*) | Startup grant (below) | ~75 ms Flash (*reported*) | `elevenlabs`, `ELEVENLABS_API_KEY` |
| Google Cloud TTS | Standard ~$4 per 1M characters (*reported*) | Monthly free allowance (amounts disagree between sources) | – | Not built in yet |
| Azure Speech | ~$16 per 1M neural characters (*reported*) | 500,000 neural characters/month, no expiry (*reported*) | – | Not built in yet |
| Amazon Polly | ~$4 per 1M standard characters (*reported*) | First 12 months on older accounts; newer accounts may get credits instead (*reported*) | – | Not built in yet |

Not used: OpenAI's realtime voice through Codex. It is a speech-to-speech
model on the user's ChatGPT plan (what Zeron uses), but it is gated by OpenAI's
rollout and was not available on this project's test account (the call is
refused with 404 at `chatgpt.com/backend-api/codex/realtime/calls`). Keplorayx can
add it once accounts have it.

## Free and startup programs

| Program | Offer | Who | Source |
| --- | --- | --- | --- |
| Fish Audio Startup Program | 3 months, 12M characters a month, raised concurrency | Early-stage teams building companions, characters or agents | [fish.audio/startup](https://fish.audio/startup) |
| ElevenLabs Startup Grants | 33M characters (~680 hours of Agents) over 12 months | Under 25 employees, business plan, long-term product | [elevenlabs.io/startup-grants](https://elevenlabs.io/startup-grants) |
| Fish Audio free model | `s2.1-pro-free`, free for developers | Anyone with an API key | [fish.audio/developers](https://fish.audio/developers/) |
| Deepgram | $200 sign-up credit, no card; a startup program of up to $100K over 12 months is *reported* (its official page wasn't reachable when checked) | Startups (*reported*) | [pricing](https://texttolab.com/blog/deepgram-pricing) |
| Cartesia free plan | 20,000 credits a month | Non-commercial use (*reported*) | [pricing guide](https://texttolab.com/blog/cartesia-pricing) |
| Google for Startups Cloud | Up to $200K, $350K for AI-first startups; meaningful tiers need funding (*reported*) | Funded startups | [comparison](https://fin.ai/learn/startup-credit-programs) |
| Microsoft for Startups | $200 self-serve, rising with usage up to $150K; investor-referred from $100K (*reported*) | Startups | [comparison](https://fin.ai/learn/startup-credit-programs) |
| AWS Activate | $1K–$5K self-funded; up to $100K through a VC/accelerator (*reported*) | Startups | [comparison](https://fin.ai/learn/startup-credit-programs) |
| AssemblyAI (speech recognition) | $50 credit (*reported*); a startup program for bootstrapped through Series A (*reported*) | Startups | [tracker](https://agentdeals.dev/vendor/assemblyai) |

For Keplorayx itself the best mix today: the Fish Audio free model for speech,
then apply to the Fish Audio and ElevenLabs startup programs for headroom.

## Speech recognition

Keplorayx recognises speech locally, so none of this is required.

- **Moonshine base** (default for Voice Mode and live dictation): MIT, 66 MB
  download, ~160 MB in use, ~290 ms per turn on the test machine.
- **Parakeet TDT 0.6B v3**: more accurate, ~700 MB in use, CC-BY-4.0.
- **Moonshine v2 streaming** (tiny/small/medium, Feb 2026, MIT for English)
  uses a sliding-window encoder made for live captions ([paper](https://arxiv.org/abs/2602.12241),
  [model docs](https://huggingface.co/docs/transformers/en/model_doc/moonshine_streaming)).
  The official repositories publish no ONNX export yet, only community
  conversions, so Keplorayx doesn't ship it; it is the next step for even faster
  live dictation.
- Cloud options already in Keplorayx: Deepgram (`DEEPGRAM_API_KEY`) and any
  OpenAI-compatible `/audio/transcriptions` endpoint.

## Setting it up

1. Make an account and an API key with the service.
2. Put the key in your environment (Windows: *Edit environment variables for
   your account*), for example `FISH_AUDIO_API_KEY`, and restart Keplorayx. Keplorayx
   reads keys from the environment and never stores them.
3. Settings › Voice › Cloud voice: turn it on and pick the service. *Service
   voice* takes that service's voice id (empty uses Keplorayx's default).
4. Start a call. If the service fails (offline, out of credits, key revoked),
   Keplorayx says so once and keeps speaking with your system's voice.

Live check of a key from a terminal (sends one short sentence):

```bash
cargo test -p dove-voice --release live_providers -- --ignored --nocapture
```
