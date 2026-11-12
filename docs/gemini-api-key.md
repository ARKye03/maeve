# Get a Gemini API key

Maeve uses Google's Gemini models to argue for each product and pick a winner, so it needs a Gemini API key of your own. Creating one is free and takes a couple of minutes.

1. Open [Google AI Studio](https://aistudio.google.com/apikey) and sign in with your Google account.
2. Click **Create API key**. If AI Studio asks for a Google Cloud project, pick an existing one or let it create one.
3. Copy the key. It starts with `AIza`.
4. In Maeve, open **Ajustes** (Settings), paste the key under **Clave de API** and click **Guardar clave**.

Maeve sends the key to Gemini once to confirm it works, then saves it in the macOS Keychain. The key never goes into Maeve's settings file. To replace it, click **Cambiar clave**.

## Free tier limits

The free tier caps how many requests you can make per minute. If comparisons keep stalling on "Gemini está ocupado, reintentando", lower **Solicitudes por minuto** in Ajustes to stay under your limit. Google publishes the current numbers on its [rate limits page](https://ai.google.dev/gemini-api/docs/rate-limits).

On the free tier, Google may use what you send to improve its products. Product listings are public anyway, but don't paste anything private into a comparison. The [Gemini API terms](https://ai.google.dev/gemini-api/terms) have the details.

## Keep the key private

Anyone holding your key can spend your quota. Don't commit it or share it. If it leaks, delete it in AI Studio and create a new one.
