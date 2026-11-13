# Providers

Remote provider adapters. Each family owns its endpoint, request form and response parser and implements the capability traits declared by `text`, `recognition` and `speech`.

| Module | Contract |
|---|---|
| `anthropic/` | [Anthropic translator](anthropic/README.md) |
| `elevenlabs/` | [ElevenLabs transcription and speech synthesis](elevenlabs/README.md) |
| `openai/` | [OpenAI adapters](openai/README.md) |
