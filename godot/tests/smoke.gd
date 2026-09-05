extends SceneTree
## Headless smoke test for the GDExtension:
##   godot --headless --path godot -s tests/smoke.gd
## Exercises the resource API and the mix path via AudioStreamPlayback.mix_audio().

var _failures := 0


func _check(cond: bool, msg: String) -> void:
	if cond:
		print("  ok   ", msg)
	else:
		_failures += 1
		printerr("  FAIL ", msg)


func _peak(frames: PackedVector2Array) -> float:
	var m := 0.0
	for f in frames:
		m = maxf(m, maxf(absf(f.x), absf(f.y)))
	return m


func _init() -> void:
	print("SynthPatch")
	_check(ClassDB.class_exists("SynthPatch"), "class registered")
	var names := SynthPatch.get_param_names()
	_check(names.size() > 50, "param names: %d" % names.size())
	var patch := SynthPatch.new()
	_check(patch.set_param("filter/cutoff_hz", 1234.0), "set_param known")
	_check(is_equal_approx(patch.get_param("filter/cutoff_hz"), 1234.0), "get_param round trip")
	_check(not patch.set_param("nope/nothing", 1.0), "set_param unknown returns false")
	_check(patch.get("filter/cutoff_hz") == 1234.0, "dynamic property get")
	patch.set("osc1/wave", 5)
	_check(patch.get_param("osc1/wave") == 5.0, "dynamic property set")
	var json := patch.to_json()
	var copy := SynthPatch.from_json_string(json)
	_check(copy != null and copy.get_param("filter/cutoff_hz") == 1234.0, "json round trip")
	var laser := SynthPatch.from_preset("Laser", 7)
	var laser2 := SynthPatch.from_preset("Laser", 7)
	_check(laser.to_json() == laser2.to_json(), "preset deterministic per seed")
	_check(laser.get_length() > 0.0, "one-shot length %.2fs" % laser.get_length())
	var dup := laser.duplicate() as SynthPatch
	_check(dup.to_json() == laser.to_json(), "duplicate() copies dynamic properties")

	print("SynthStream one-shot")
	var stream := SynthStream.from_preset("Pickup", 3)
	_check(stream.patch != null, "from_preset assigns patch")
	_check(stream.get_length() > 0.0, "stream length %.2fs" % stream.get_length())
	var pb := stream.instantiate_playback()
	_check(pb is SynthStreamPlayback, "instantiate_playback type")
	pb.start(0.0)
	_check(pb.is_playing(), "playing after start")
	var frames := pb.mix_audio(1.0, 4096)
	_check(frames.size() == 4096, "mix returned %d frames" % frames.size())
	var pk := _peak(frames)
	_check(pk > 0.05 and pk <= 1.0, "one-shot peak %.3f" % pk)
	var total := 4096
	while pb.is_playing() and total < 48000 * 10:
		frames = pb.mix_audio(1.0, 4096)
		total += 4096
	_check(not pb.is_playing(), "one-shot finished after %.2fs" % (total / 48000.0))

	print("SynthStream instrument")
	var inst := SynthStream.new()
	inst.one_shot = false
	inst.patch = SynthPatch.new()
	var ipb := inst.instantiate_playback() as SynthStreamPlayback
	ipb.start(0.0)
	frames = ipb.mix_audio(1.0, 2048)
	_check(_peak(frames) == 0.0, "silent before notes")
	ipb.note_on(60, 1.0)
	ipb.note_on(64, 1.0)
	frames = ipb.mix_audio(1.0, 4096)
	_check(_peak(frames) > 0.1, "sounding after note_on, peak %.3f" % _peak(frames))
	_check(ipb.get_active_voices() == 2, "active voices == 2")
	# Live patch edit reaches the running playback.
	inst.patch.set_param("master/gain", 0.0)
	frames = ipb.mix_audio(1.0, 2048)
	frames = ipb.mix_audio(1.0, 2048)
	_check(_peak(frames) == 0.0, "live patch edit (gain 0) applied")
	inst.patch.set_param("master/gain", 0.5)
	ipb.set_param("filter/cutoff_hz", 300.0)
	ipb.note_off(60)
	ipb.note_off(64)
	total = 0
	while ipb.get_active_voices() > 0 and total < 48000 * 5:
		frames = ipb.mix_audio(1.0, 4096)
		total += 4096
	_check(ipb.get_active_voices() == 0, "voices released")
	_check(ipb.is_playing(), "instrument keeps playing while silent")
	ipb.stop()
	_check(not ipb.is_playing(), "stopped")

	print("JetEngine")
	_check(ClassDB.class_exists("JetEngineStream"), "class registered")
	var jp := JetEnginePatch.from_preset("Heavy")
	_check(jp.get_param("whine/hz") == 1400.0, "preset params loaded")
	_check(jp.set_param("whine/hz", 2000.0) and jp.get_param("whine/hz") == 2000.0, "set/get param")
	jp.set("preset/name", 2)
	jp.apply_preset()
	_check(jp.get_param("whine/hz") == 3400.0, "apply_preset via dynamic property")
	var jj := JetEnginePatch.new()
	_check(jj.from_json(jp.to_json()) and jj.get_param("whine/hz") == 3400.0, "jet json round trip")
	var js := JetEngineStream.from_preset("Racer")
	_check(js.patch != null, "from_preset assigns patch")
	var jpb := js.instantiate_playback() as JetEnginePlayback
	_check(jpb != null, "instantiate_playback type")
	jpb.start(0.0)
	var idle := jpb.mix_audio(1.0, 48000)
	var idle_pk := _peak(idle.slice(24000))
	_check(idle_pk > 0.02 and idle_pk <= 1.0, "idle audible, peak %.3f" % idle_pk)
	var rpm0 := jpb.get_rpm()
	jpb.set_state(1.0, 1.0, 1.0, 0.0)
	var full := jpb.mix_audio(1.0, 48000 * 3)
	var full_pk := _peak(full.slice(48000 * 2))
	_check(jpb.get_rpm() > rpm0 + 0.5, "rpm spooled from %.2f to %.2f" % [rpm0, jpb.get_rpm()])
	_check(full_pk > idle_pk, "full throttle louder (%.3f > %.3f)" % [full_pk, idle_pk])
	js.patch.set_param("master/gain", 0.0)
	jpb.mix_audio(1.0, 4096)
	var muted := jpb.mix_audio(1.0, 4096)
	_check(_peak(muted) == 0.0, "live jet patch edit applied")
	_check(jpb.is_playing(), "engine keeps running")
	jpb.stop()
	_check(not jpb.is_playing(), "engine stopped")

	print("Result: %d failure(s)" % _failures)
	quit(1 if _failures > 0 else 0)
