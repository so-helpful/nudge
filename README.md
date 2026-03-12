# nudge

A text processing binary that converts speech-to-text input into better context aware messages using your agent conversation history.

## How It Works

1. Ask your agent to run your audio messages through nudge.
2. nudge receives the audio
3. Whisper transcribes the audio
4. nudge processes the text using:
   - Built-in mappings from convo context
   - Fuzzy matching for corrections
   - Learned mappings from previous corrections

## Usage

```
# when you send audio
nudge audo-bytes # returns corrected text

# when the agent sends a response, it's recorded as context with
nudge -r <Response>
```
