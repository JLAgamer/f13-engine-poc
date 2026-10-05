"""Extract gameplay tuning from YOUR OWN install of Friday the 13th: The Game into a local tuning.json.

Nothing from the game is stored in this repository; this script reads your install and writes the values to a
file on your machine (default: ~/.f13-rs/tuning.json). Your install is never modified: assets are copied
to a temp folder before conversion.

Requirements: Python 3.10+, UAssetGUI (https://github.com/atenfyr/UAssetGUI) for its `tojson` command, and an
install whose main pak is extracted to loose files (the usual modding setup).

Usage:
  python tools/extract_tuning.py --game "C:/Program Files (x86)/Steam/steamapps/common/F13Game" \
      --uassetgui path/to/UAssetGUI.exe [--out tuning.json]
"""
import argparse, json, os, shutil, struct, subprocess, sys, tempfile

# C++-only defaults, read from the game executable. Each entry pins the exact instruction
# (`mov dword ptr [rdi+disp32], imm32`) so a different game build fails loudly instead of yielding garbage.
EXE_CONSTANTS = {
    # name: (virtual address, expected instruction bytes without the immediate)
    "rage_unlock_time": (0x1403E0CC0, bytes.fromhex("c78750130000")),
    "stalk_active_time": (0x1403E0CB6, bytes.fromhex("c78700170000")),
    "update_fear_delay": (0x14032CA2C, bytes.fromhex("c78740010000")),
    # character movement component constructor
    "max_sprint_speed": (0x1403A2A45, bytes.fromhex("c78320070000")),
    "max_run_speed": (0x1403A2A7C, bytes.fromhex("c78324070000")),
    "max_slow_run_speed": (0x1403A2A86, bytes.fromhex("c78328070000")),
    "max_combat_speed": (0x1403A2A90, bytes.fromhex("c7832c070000")),
    "max_walk_speed": (0x1403A2A9A, bytes.fromhex("c783d4010000")),
    "max_walk_speed_crouched": (0x1403A2AA4, bytes.fromhex("c783d8010000")),
    # counselor constructor (values a Blueprint may override; the Blueprint wins)
    "stamina_max": (0x1403A095A, bytes.fromhex("c787dc120000")),
    "stamina_idle_modifier": (0x1403A0984, bytes.fromhex("c78720130000")),
    "stamina_hiding_modifier": (0x1403A098E, bytes.fromhex("48c78724130000")),
    "stamina_sprinting_modifier": (0x1403A09A3, bytes.fromhex("c78730130000")),
    "dodging_speed_mod": (0x1403A0A83, bytes.fromhex("c78788140000")),
    "stumbling_speed_mod": (0x1403A0A8D, bytes.fromhex("c7878c140000")),
    "water_speed_mod": (0x1403A0A97, bytes.fromhex("c78790140000")),
    "grabbed_refill_base": (0x1403A13B0, bytes.fromhex("c78770130000")),
    "grabbed_refill_strength": (0x1403A13BA, bytes.fromhex("c78774130000")),
    "grabbed_refill_composure": (0x1403A13C4, bytes.fromhex("c78788130000")),
    "stumble_wounded_chance": (0x1403A0D08, bytes.fromhex("c787c8190000")),
    "stumble_cooldown": (0x1403A0D19, bytes.fromhex("c787cc190000")),
    "stumble_stat_mod_base": (0x1403A145A, bytes.fromhex("c78798190000")),
}
EXE_FLAGS = {  # name: (virtual address, instruction bytes without the 1-byte immediate)
    "force_stumble_on_stamina_depletion": (0x1403A0D23, bytes.fromhex("c687d0190000")),
}

FEAR_FIELDS = {  # asset property -> json key; properties not set in an asset are 0 (no C++ constructor)
    "IncreaseFearValue": "increase", "DecreaseFearValue": "decrease", "IncreaseDelay": "increase_delay",
    "DecreaseDelay": "decrease_delay", "MaxFearCap": "max_fear_cap", "bDecreaseWhenActive": "decrease_when_active",
    "AutoDeactivateTimer": "auto_deactivate_time",
}
STAT_NAMES = ["Strength", "Speed", "Intelligence", "Stamina", "Luck", "Composure", "Stealth"]


def read_exe_constants(exe):
    data = open(exe, "rb").read()
    pe = struct.unpack_from("<I", data, 0x3C)[0]
    image_base = struct.unpack_from("<Q", data, pe + 24 + 24)[0]
    nsec = struct.unpack_from("<H", data, pe + 6)[0]
    sec0 = pe + 24 + struct.unpack_from("<H", data, pe + 20)[0]
    secs = [struct.unpack_from("<8sIIII", data, sec0 + 40 * i) for i in range(nsec)]

    def file_off(va):
        rva = va - image_base
        for _, vsize, vaddr, rsize, raddr in secs:
            if vaddr <= rva < vaddr + max(vsize, rsize):
                return raddr + rva - vaddr
        raise ValueError(hex(va))

    out = {}
    for name, (va, expect) in EXE_CONSTANTS.items():
        o = file_off(va)
        if data[o:o + len(expect)] != expect:
            sys.exit(f"{name}: unexpected instruction at {va:#x}; this game build is not supported yet")
        out[name] = struct.unpack_from("<f", data, o + len(expect))[0]
    for name, (va, expect) in EXE_FLAGS.items():
        o = file_off(va)
        if data[o:o + len(expect)] != expect:
            sys.exit(f"{name}: unexpected instruction at {va:#x}; this game build is not supported yet")
        out[name] = data[o + len(expect)] != 0
    return out


class Converter:
    def __init__(self, content, tool):
        self.content, self.tool = content, tool
        self.tmp = tempfile.mkdtemp(prefix="f13tuning_")

    def load(self, rel):
        """rel like 'Characters/Killers/Jason/Base/Jason_Base' -> parsed UAssetAPI JSON."""
        src = os.path.join(self.content, rel)
        dst = os.path.join(self.tmp, rel.replace("/", "_"))
        for ext in (".uasset", ".uexp"):
            if os.path.exists(src + ext):
                shutil.copy2(src + ext, dst + ext)
        out = dst + ".json"
        subprocess.run([self.tool, "tojson", dst + ".uasset", out, "VER_UE4_18"], check=True, capture_output=True)
        return json.load(open(out, encoding="utf-8"))

    def close(self):
        shutil.rmtree(self.tmp, ignore_errors=True)


def num(v):
    return float(v) if isinstance(v, (int, float)) else float(str(v).replace("+", ""))


def props(export):
    """{(name, index): value-node} for an export's tagged properties."""
    return {(p["Name"], p.get("ArrayIndex", 0)): p for p in (export.get("Data") or [])}


def export_named(asset, pred):
    return next(e for e in asset["Exports"] if pred(e["ObjectName"]))


def obj_name(asset, idx):
    if idx < 0:
        return asset["Imports"][-idx - 1]["ObjectName"]
    if idx > 0:
        return asset["Exports"][idx - 1]["ObjectName"]
    return None


def obj_path(asset, idx):
    """Content-relative package path (no extension) of an imported object, e.g. 'Blueprints/StatsCurves/X'."""
    imp = asset["Imports"][-idx - 1]
    while imp["OuterIndex"] != 0:
        imp = asset["Imports"][-imp["OuterIndex"] - 1]
    return imp["ObjectName"].replace("/Game/", "", 1)


def montage_length(conv, rel):
    """Play length of an AnimMontage at rate 1 (what PlayAnimMontage returns)."""
    asset = conv.load(rel)
    p = props(export_named(asset, lambda n: not n.startswith("Default__")))
    rate = num(p[("RateScale", 0)]["Value"]) if ("RateScale", 0) in p else 1.0
    return num(p[("SequenceLength", 0)]["Value"]) / rate


def stat_value(node, base=0.0):
    """StatModifiedValue struct node -> {"base", "modifiers"[7]} (unset fields are 0)."""
    mods = [0.0] * 7
    for x in node["Value"]:
        if x["Name"] == "BaseValue":
            base = num(x["Value"])
        elif x["Name"] == "StatModifiers":
            mods[x.get("ArrayIndex", 0)] = num(x["Value"])
    return {"base": base, "modifiers": mods}


def counselor_section(conv, consts):
    base = conv.load("Characters/Counselors/Base/Base_Counselor")
    d = props(export_named(base, lambda n: n.startswith("Default__")))
    move = props(export_named(base, lambda n: n == "CharMoveComp"))

    def f(name, fallback):
        return num(d[(name, 0)]["Value"]) if (name, 0) in d else fallback

    def smv(name, base=0.0):  # base: the C++ constructor's BaseValue, kept when the Blueprint doesn't override it
        return stat_value(d[(name, 0)], base) if (name, 0) in d else {"base": base, "modifiers": [0.0] * 7}

    curves = {}

    def curve(name):
        """Curve property -> curve name (its keys go into `curves`), or None when unset."""
        if (name, 0) not in d or not d[(name, 0)]["Value"]:
            return None
        path = obj_path(base, d[(name, 0)]["Value"])
        cname = path.rsplit("/", 1)[-1]
        if cname not in curves:
            curves[cname] = curve_keys(conv.load(path))
        return cname

    def montage(name):
        return montage_length(conv, obj_path(base, d[(name, 0)]["Value"])) if (name, 0) in d and d[(name, 0)]["Value"] else 0.0

    fear_curve = curve("StaminaFearRefillModifier")
    grabbed = {"base": consts["grabbed_refill_base"], "modifiers": [0.0] * 7}
    grabbed["modifiers"][0] = consts["grabbed_refill_strength"]
    grabbed["modifiers"][5] = consts["grabbed_refill_composure"]
    tuning = {
        "max_walk_speed": consts["max_walk_speed"],
        "max_walk_speed_crouched": consts["max_walk_speed_crouched"],
        "max_slow_run_speed": consts["max_slow_run_speed"],
        "max_run_speed": consts["max_run_speed"],
        "max_sprint_speed": consts["max_sprint_speed"],
        "max_combat_speed": consts["max_combat_speed"],
        "max_swim_speed": num(move[("MaxSwimSpeed", 0)]["Value"]),
        "max_acceleration": num(move[("MaxAcceleration", 0)]["Value"]),
        "dodging_speed_mod": f("DodgingSpeedMod", consts["dodging_speed_mod"]),
        "stumbling_speed_mod": f("StumblingSpeedMod", consts["stumbling_speed_mod"]),
        "water_speed_mod": f("WaterSpeedMod", consts["water_speed_mod"]),
        "speed_stat_mod": smv("SpeedStatMod"),
        "stamina_max": consts["stamina_max"],
        "stamina_min_threshold": f("StaminaMinThreshold", 0.0),
        "stamina_deplete_rate": f("StaminaDepleteRate", 0.0),
        "stamina_refill_rate": smv("StaminaRefillRate"),
        "stamina_idle_modifier": f("StaminaIdleModifier", consts["stamina_idle_modifier"]),
        "stamina_hiding_modifier": f("StaminaHidingModifier", consts["stamina_hiding_modifier"]),
        "stamina_walking_modifier": f("StaminaWalkingModifier", 0.0),
        "stamina_running_modifier": f("StaminaRunningModifier", 0.0),
        "stamina_sprinting_modifier": f("StaminaSprintingModifier", consts["stamina_sprinting_modifier"]),
        "stamina_swimming_modifier": f("StaminaSwimmingModifier", 0.0),
        "stamina_crouching_modifier": f("StaminaCrouchingModifier", 0.0),
        "stamina_stat_mod": smv("StaminaStatMod"),
        "grabbed_stamina_refill": grabbed,
        "stamina_fear_curve": fear_curve,
        "force_stumble_on_stamina_depletion": consts["force_stumble_on_stamina_depletion"],
        "wounded_threshold": f("WoundedThreshold", 0.0),
        "jason_near_min_range": f("JasonNearMinRange", 0.0),
        "jason_near_max_range": f("JasonNearMaxRange", 0.0),
        "stumble": {
            "fear_frequency_curve": curve("StumbleFearFrequecy"),  # (sic) the property's name in the game
            "stamina_frequency_curve": curve("StumbleStaminaFrequency"),
            "fear_chance_curve": curve("StumbleFearChance"),
            "stamina_chance_curve": curve("StumbleStaminaChance"),
            "stat_mod": smv("StumbleStatMod", consts["stumble_stat_mod_base"]),
            "wounded_chance": f("StumbleWoundedChance", consts["stumble_wounded_chance"]),
            "cooldown": f("StumbleCooldown", consts["stumble_cooldown"]),
            "sprint_length": montage("SprintStumbleAnim"),
            "run_length": montage("RunStumbleAnim"),
            "walk_length": montage("WalkStumbleAnim"),
            "out_of_stamina_length": montage("OutOfStaminaStumbleAnim"),
        },
    }
    # Playable counselors in the MVP: stats from each character Blueprint
    characters = {}
    for key, rel in {"Vanessa": "Characters/Counselors/Athlete/Athlete_Counselor"}.items():
        a = conv.load(rel)
        p = props(export_named(a, lambda n: n.startswith("Default__")))
        characters[key] = {"stats": [int(num(p[("Stats", i)]["Value"])) for i in range(7)]}
    return tuning, characters, curves


def curve_keys(asset):
    e = asset["Exports"][0]
    rc = next(p for p in e["Data"] if p["Name"] == "FloatCurve")
    keys_arr = next(p for p in rc["Value"] if p["Name"] == "Keys")
    out = []
    for k in keys_arr["Value"]:
        v = k["Value"][0]["Value"]
        out.append({
            "time": num(v["Time"]), "value": num(v["Value"]),
            "interp": {"RCIM_Linear": "linear", "RCIM_Constant": "constant"}.get(v["InterpMode"], "cubic"),
            "arrive_tangent": num(v["ArriveTangent"]), "leave_tangent": num(v["LeaveTangent"]),
        })
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--game", required=True)
    ap.add_argument("--uassetgui", required=True)
    ap.add_argument("--out", default=os.path.join(os.path.expanduser("~"), ".f13-rs", "tuning.json"))
    a = ap.parse_args()
    content = os.path.join(a.game, "SummerCamp", "Content")
    if not os.path.isfile(os.path.join(content, "Characters", "Killers", "Jason", "Base", "Jason_Base.uasset")):
        paks = os.path.join(content, "Paks")
        packed = os.path.isdir(paks) and any(f.lower().endswith(".pak") for f in os.listdir(paks))
        sys.exit(f'"{a.game}" is a stock install: its content is still packed in Content/Paks/*.pak.\n'
                 "Extract the main pak to loose files under SummerCamp/Content first (the usual modding setup)."
                 if packed else f'"{a.game}" has no extracted game content under SummerCamp/Content.')
    exe = os.path.join(a.game, "SummerCamp", "Binaries", "Win64", "SummerCamp.exe")
    consts = read_exe_constants(exe)
    conv = Converter(content, a.uassetgui)
    try:
        # Jason (base Blueprint)
        jason = conv.load("Characters/Killers/Jason/Base/Jason_Base")
        p = props(export_named(jason, lambda n: n.startswith("Default__")))
        killer = {
            "unlock_time": [num(p[("AbilityUnlockTime", i)]["Value"]) for i in range(4)],
            "recharge_time": [num(p[("AbilityRechargeTime", i)]["Value"]) for i in range(4)],
            "raged_recharge_modifier": num(p[("RagedRechargeTimeModifier", 0)]["Value"]),
            "rage_unlock_time": consts["rage_unlock_time"],
        }
        # Fear events
        fear_events, curves = {}, {}
        fe_dir = os.path.join(content, "Blueprints", "FearEvents")
        for f in sorted(os.listdir(fe_dir)):
            if not f.endswith(".uasset"):
                continue
            name = f[:-7]
            asset = conv.load(f"Blueprints/FearEvents/{name}")
            p = props(export_named(asset, lambda n: n.startswith("Default__")))
            ev = {k: (bool(p[(src, 0)]["Value"]) if k == "decrease_when_active" else num(p[(src, 0)]["Value"]))
                  if (src, 0) in p else (False if k == "decrease_when_active" else 0.0)
                  for src, k in FEAR_FIELDS.items()}
            ev["distance_curve"] = None
            if ("FearDistanceCurve", 0) in p:
                cname = obj_name(asset, p[("FearDistanceCurve", 0)]["Value"])
                ev["distance_curve"] = cname
                if cname not in curves:
                    # curve assets live next to the counselors in the stock layout
                    curves[cname] = curve_keys(conv.load(f"Characters/Counselors/{cname}"))
            fear_events[name] = ev
        # Counselor fear modifier (FearComponent template on the base counselor)
        counselor = conv.load("Characters/Counselors/Base/Base_Counselor")
        fc = export_named(counselor, lambda n: n == "FearComponent")
        fm = next(x for x in fc["Data"] if x["Name"] == "FearModifier")
        mods = [0.0] * 7
        base_value = 1.0
        for x in fm["Value"]:
            if x["Name"] == "BaseValue":
                base_value = num(x["Value"])
            elif x["Name"] == "StatModifiers":
                mods[x.get("ArrayIndex", 0)] = num(x["Value"])
        counselor, characters, counselor_curves = counselor_section(conv, consts)
        curves.update(counselor_curves)
        out = {
            "format": 3,
            "counselor": {"Base_Counselor": counselor, "characters": characters},
            "killer": {"Jason_Base": killer, "stalk_active_time": consts["stalk_active_time"]},
            "fear": {
                "update_fear_delay": consts["update_fear_delay"],
                "fear_modifier": {"base": base_value, "modifiers": mods, "stat_order": STAT_NAMES},
                "events": fear_events,
                "curves": curves,
            },
        }
    finally:
        conv.close()
    os.makedirs(os.path.dirname(os.path.abspath(a.out)), exist_ok=True)
    json.dump(out, open(a.out, "w"), indent=1)
    print(f"wrote {a.out}: {len(fear_events)} fear events, {len(curves)} curves")


if __name__ == "__main__":
    main()
