extends SceneTree
## One-shots must END from Godot's point of view: play them through real AudioStreamPlayers in
## the scene tree and await `finished`. The AudioServer ends a playback only when mix() returns
## fewer frames than it asked for, so this is the test that catches a playback which goes quiet
## but keeps returning full buffers. Works headless: the dummy audio driver mixes in real time.
##   godot --headless --path godot -s tests/finished.gd

var _failures := 0


func _check(cond: bool, msg: String) -> void:
	if cond:
		print("  ok   ", msg)
	else:
		_failures += 1
		printerr("  FAIL ", msg)


func _initialize() -> void:
	_run.call_deferred()


## Seconds until `finished`, or -1 if it did not arrive within `timeout`.
func _play_and_wait(stream: AudioStream, timeout: float) -> float:
	var player := AudioStreamPlayer.new()
	root.add_child(player)
	player.stream = stream
	var state := {"done": false}
	player.finished.connect(func() -> void: state.done = true)
	var start := Time.get_ticks_msec()
	player.play()
	while not state.done and Time.get_ticks_msec() - start < timeout * 1000.0:
		await process_frame
	var took := (Time.get_ticks_msec() - start) / 1000.0
	player.queue_free()
	return took if state.done else -1.0


func _expect_finished(label: String, stream: AudioStream, at_least: float, at_most: float) -> void:
	var took: float = await _play_and_wait(stream, at_most)
	_check(took >= at_least, "%s: `finished` after %.2f s (expected %.2f .. %.2f s)" % [label, took, at_least, at_most])


func _run() -> void:
	await process_frame
	print("Players emit `finished`")
	# Control: if a plain sample does not finish, the environment is at fault, not gamesynth.
	var wav := AudioStreamWAV.new()
	wav.format = AudioStreamWAV.FORMAT_16_BITS
	wav.mix_rate = 44100
	var silence := PackedByteArray()
	silence.resize(44100 * 2 / 5)
	wav.data = silence
	await _expect_finished("control: 0.2 s AudioStreamWAV", wav, 0.1, 3.0)

	var pickup := SoundGenerator.create("pickup")
	_check(pickup.get_length() > 0.3 and pickup.get_length() < 5.0, "pickup reports its length: %.2f s" % pickup.get_length())
	await _expect_finished("SoundGenerator pickup", pickup, 0.2, pickup.get_length() + 1.0)
	await _expect_finished("SoundGenerator pickup, played again", pickup, 0.2, pickup.get_length() + 1.0)

	var blast := SoundGenerator.create("mine_blast")
	blast.set_start_input("power", 0.5)
	await _expect_finished("SoundGenerator mine_blast at power 0.5", blast, 0.3, blast.get_length() + 1.0)

	var chime := SoundGenerator.from_file(ProjectSettings.globalize_path("res://").path_join("../models/checkpoint.toml"))
	_check(chime.get_error() == "" and chime.get_length() > 0.3, "model-file one-shot reports a measured length: %.2f s" % chime.get_length())
	await _expect_finished("SoundGenerator from checkpoint.toml", chime, 0.2, chime.get_length() + 1.0)

	_check(SoundGenerator.create("wind").get_length() == 0.0, "continuous generators report length 0")
	_check(SoundGenerator.create("explosion").get_length() > 1.5, "explosion length includes its tail: %.2f s" % SoundGenerator.create("explosion").get_length())

	var blip := SynthStream.from_preset("Blip", 1)
	await _expect_finished("SynthStream preset one-shot", blip, 0.05, blip.get_length() + 1.5)

	# Amp sustain 0 and no master/duration: reported from the game as never ending.
	var patch := SynthPatch.new()
	patch.set_param("osc1/wave", 0)
	patch.set_param("amp_env/attack", 0.003)
	patch.set_param("amp_env/decay", 0.3)
	patch.set_param("amp_env/sustain", 0.0)
	var decaying := SynthStream.new()
	decaying.patch = patch
	decaying.one_shot = true
	_check(patch.get_param("master/duration") == 0.0, "sustain-0 patch has no master/duration")
	await _expect_finished("SynthStream sustain 0, no duration", decaying, 0.2, 3.0)

	print("Result: %d failure(s)" % _failures)
	quit(1 if _failures > 0 else 0)
