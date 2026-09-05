extends Control
## GameSynth demo: sound-effect presets on buttons, plus a keyboard-playable instrument.
##
## SFX: each button creates a one-shot SynthStream from a preset + seed and plays it on a
## pooled AudioStreamPlayer, so several effects can overlap.
## Instrument: one SynthStream with one_shot = false stays running; keys A W S E D F T G Y H U J K
## send note_on / note_off to its SynthStreamPlayback.
## Jet engine: one JetEngineStream driven every frame from sliders / keys (Up = throttle,
## Shift = boost), the way a ship script would from its physics state.

const KEYS := {
	KEY_A: 60, KEY_W: 61, KEY_S: 62, KEY_E: 63, KEY_D: 64, KEY_F: 65, KEY_T: 66,
	KEY_G: 67, KEY_Y: 68, KEY_H: 69, KEY_U: 70, KEY_J: 71, KEY_K: 72,
}

var _seed := 0
var _sfx_players: Array[AudioStreamPlayer] = []
var _instrument_player: AudioStreamPlayer
var _instrument_patch: SynthPatch
var _held := {}
var _status: Label

var _jet_player: AudioStreamPlayer
var _jet_patch: JetEnginePatch
var _jet_throttle: HSlider
var _jet_boost: HSlider
var _jet_speed: HSlider
var _jet_damage: HSlider
var _jet_rpm: Label
var _key_throttle := false
var _key_boost := false


func _ready() -> void:
	var root := VBoxContainer.new()
	root.set_anchors_and_offsets_preset(PRESET_FULL_RECT, PRESET_MODE_MINSIZE, 16)
	add_child(root)

	# --- Sound effects -------------------------------------------------------------------
	var title := Label.new()
	title.text = "Sound effects (procedural, seed-based)"
	root.add_child(title)

	var seed_row := HBoxContainer.new()
	root.add_child(seed_row)
	var seed_label := Label.new()
	seed_label.text = "Seed"
	seed_row.add_child(seed_label)
	var seed_box := SpinBox.new()
	seed_box.max_value = 99999
	seed_box.value_changed.connect(func(v: float) -> void: _seed = int(v))
	seed_row.add_child(seed_box)
	var random_btn := Button.new()
	random_btn.text = "Random seed"
	random_btn.pressed.connect(func() -> void: seed_box.value = randi() % 100000)
	seed_row.add_child(random_btn)

	var grid := GridContainer.new()
	grid.columns = 4
	root.add_child(grid)
	for preset in SynthPatch.get_preset_names():
		var b := Button.new()
		b.text = preset
		b.custom_minimum_size = Vector2(160, 40)
		b.pressed.connect(_play_sfx.bind(preset))
		grid.add_child(b)

	for i in 8:
		var p := AudioStreamPlayer.new()
		add_child(p)
		_sfx_players.append(p)

	# --- Instrument ----------------------------------------------------------------------
	var sep := HSeparator.new()
	root.add_child(sep)
	var inst_title := Label.new()
	inst_title.text = "Instrument: play with keys A W S E D F T G Y H U J K (Z/X = octave)"
	root.add_child(inst_title)

	_instrument_patch = SynthPatch.new()
	_instrument_patch.set_param("osc1/wave", 2)  # Saw
	_instrument_patch.set_param("osc2/wave", 2)
	_instrument_patch.set_param("osc2/level", 0.6)
	_instrument_patch.set_param("osc2/detune_cents", 12)
	_instrument_patch.set_param("filter/cutoff_hz", 900)
	_instrument_patch.set_param("filter/resonance", 0.4)
	_instrument_patch.set_param("filter/env_amount", 3.0)
	_instrument_patch.set_param("filter_env/decay", 0.4)
	_instrument_patch.set_param("amp_env/release", 0.4)
	_instrument_patch.set_param("fx/delay_time", 0.3)
	_instrument_patch.set_param("fx/delay_mix", 0.25)

	var stream := SynthStream.new()
	stream.patch = _instrument_patch
	stream.one_shot = false
	_instrument_player = AudioStreamPlayer.new()
	_instrument_player.stream = stream
	add_child(_instrument_player)
	_instrument_player.play()

	_add_slider(root, "Cutoff", "filter/cutoff_hz", 100, 12000, true)
	_add_slider(root, "Resonance", "filter/resonance", 0, 1, false)
	_add_slider(root, "Release", "amp_env/release", 0.01, 3, false)
	_add_slider(root, "Delay mix", "fx/delay_mix", 0, 1, false)

	_status = Label.new()
	root.add_child(_status)

	# --- Jet engine ----------------------------------------------------------------------
	root.add_child(HSeparator.new())
	var jet_row := HBoxContainer.new()
	root.add_child(jet_row)
	var jet_title := Label.new()
	jet_title.text = "Jet engine (hold Up = throttle, Shift = boost)   preset:"
	jet_row.add_child(jet_title)
	var presets := OptionButton.new()
	for name in JetEnginePatch.get_preset_names():
		presets.add_item(name)
	jet_row.add_child(presets)
	_jet_rpm = Label.new()
	jet_row.add_child(_jet_rpm)

	_jet_patch = JetEnginePatch.from_preset("Racer")
	var jet_stream := JetEngineStream.new()
	jet_stream.patch = _jet_patch
	_jet_player = AudioStreamPlayer.new()
	_jet_player.stream = jet_stream
	_jet_player.volume_db = -6.0
	add_child(_jet_player)
	_jet_player.play()
	presets.item_selected.connect(func(i: int) -> void:
		# Swap the whole parameter set live; the running engine keeps its RPM.
		_jet_patch.from_json(JetEnginePatch.from_preset(presets.get_item_text(i)).to_json()))

	_jet_throttle = _add_plain_slider(root, "Throttle", 0.0)
	_jet_boost = _add_plain_slider(root, "Boost", 0.0)
	_jet_speed = _add_plain_slider(root, "Speed", 0.0)
	_jet_damage = _add_plain_slider(root, "Damage", 0.0)


func _add_slider(parent: Control, label: String, param: String, lo: float, hi: float, exp: bool) -> void:
	var row := HBoxContainer.new()
	parent.add_child(row)
	var l := Label.new()
	l.text = label
	l.custom_minimum_size.x = 90
	row.add_child(l)
	var s := HSlider.new()
	s.min_value = lo
	s.max_value = hi
	s.step = 0.001
	s.exp_edit = exp
	s.value = _instrument_patch.get_param(param)
	s.custom_minimum_size.x = 400
	# Editing the resource updates the running instrument live.
	s.value_changed.connect(func(v: float) -> void: _instrument_patch.set_param(param, v))
	row.add_child(s)


func _add_plain_slider(parent: Control, label: String, value: float) -> HSlider:
	var row := HBoxContainer.new()
	parent.add_child(row)
	var l := Label.new()
	l.text = label
	l.custom_minimum_size.x = 90
	row.add_child(l)
	var s := HSlider.new()
	s.max_value = 1.0
	s.step = 0.001
	s.value = value
	s.custom_minimum_size.x = 400
	row.add_child(s)
	return s


var _octave := 0

func _play_sfx(preset: String) -> void:
	var stream := SynthStream.from_preset(preset, _seed)
	# Pick a free player, or steal the oldest.
	var player: AudioStreamPlayer = _sfx_players[0]
	for p in _sfx_players:
		if not p.playing:
			player = p
			break
	player.stream = stream
	player.play()
	_status.text = "%s seed %d  (%.2fs)" % [preset, _seed, stream.patch.get_length()]


func _unhandled_key_input(event: InputEvent) -> void:
	var key := event as InputEventKey
	if key == null or key.echo:
		return
	if key.keycode == KEY_UP:
		_key_throttle = key.pressed
		return
	if key.keycode == KEY_SHIFT:
		_key_boost = key.pressed
		return
	if key.keycode == KEY_Z and key.pressed:
		_octave = max(_octave - 1, -3)
		return
	if key.keycode == KEY_X and key.pressed:
		_octave = min(_octave + 1, 3)
		return
	if not KEYS.has(key.keycode):
		return
	var playback := _instrument_player.get_stream_playback() as SynthStreamPlayback
	if playback == null:
		return
	var note: int = KEYS[key.keycode] + 12 * _octave
	if key.pressed:
		_held[key.keycode] = note
		playback.note_on(note, 0.9)
	elif _held.has(key.keycode):
		playback.note_off(_held[key.keycode])
		_held.erase(key.keycode)


func _process(delta: float) -> void:
	var playback := _instrument_player.get_stream_playback() as SynthStreamPlayback
	if playback and _held.size() > 0:
		_status.text = "voices: %d  peak: %.2f" % [playback.get_active_voices(), playback.get_peak()]

	# Keys push the sliders, like a ship's input would push its physics.
	if _key_throttle:
		_jet_throttle.value = minf(_jet_throttle.value + delta * 1.5, 1.0)
	if _key_boost:
		_jet_boost.value = minf(_jet_boost.value + delta * 4.0, 1.0)
	elif _jet_boost.value > 0.0:
		_jet_boost.value = maxf(_jet_boost.value - delta * 2.0, 0.0)
	# Speed lags throttle, roughly like a vehicle would.
	var target_speed := _jet_throttle.value * (1.0 + 0.3 * _jet_boost.value)
	_jet_speed.value = move_toward(_jet_speed.value, minf(target_speed, 1.0), delta * 0.3)

	var jet := _jet_player.get_stream_playback() as JetEnginePlayback
	if jet:
		jet.set_state(_jet_throttle.value, _jet_boost.value, _jet_speed.value, _jet_damage.value)
		_jet_rpm.text = "   RPM %3d%%" % int(jet.get_rpm() * 100.0)
