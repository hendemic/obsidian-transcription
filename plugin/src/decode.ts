// Decode any Chromium-supported audio to 16 kHz mono f32 PCM.
export const TARGET_SAMPLE_RATE = 16000;

export async function decodeTo16kMono(data: ArrayBuffer): Promise<Float32Array> {
	const ctx = new AudioContext();
	let decoded: AudioBuffer;
	try {
		// decodeAudioData detaches its input; pass a copy so the caller's buffer stays usable.
		decoded = await ctx.decodeAudioData(data.slice(0));
	} finally {
		void ctx.close();
	}
	const frames = Math.max(1, Math.ceil(decoded.duration * TARGET_SAMPLE_RATE));
	// A mono destination makes Web Audio down-mix multi-channel sources for us.
	const offline = new OfflineAudioContext(1, frames, TARGET_SAMPLE_RATE);
	const source = offline.createBufferSource();
	source.buffer = decoded;
	source.connect(offline.destination);
	source.start();
	const rendered = await offline.startRendering();
	return rendered.getChannelData(0);
}
