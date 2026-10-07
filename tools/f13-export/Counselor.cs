// counselors/<name>.json (killers/<name>.json for Jason): how a character Blueprint assembles its character (body, outfit, face, hair), the shared
// skeleton's bones with their translation retargeting modes, and the animations f13-game plays on it.
// anims/<sequence>.anim: one animation sequence sampled at its own frame rate (format below).
//
// The counselor's CharacterMesh0 is the bare base body; the default outfit (DefaultCounselorOutfit's ClothingMesh,
// with the first material option of each clothing slot) follows it as a master-pose component. The face is a separate
// skeleton (its own anim Blueprint drives the facial bones) and the hair hangs off the face.
using CUE4Parse.FileProvider;
using CUE4Parse.UE4.Assets.Exports;
using CUE4Parse.UE4.Assets.Exports.Animation;
using CUE4Parse.UE4.Assets.Exports.Material;
using CUE4Parse.UE4.Assets.Exports.SkeletalMesh;
using CUE4Parse.UE4.Objects.Core.Math;
using CUE4Parse.UE4.Objects.UObject;
using CUE4Parse.UE4.Objects.Engine;
using CUE4Parse.UE4.Objects.Engine.Curves;
using CUE4Parse.UE4.Assets.Objects;
using CUE4Parse.UE4.Assets.Exports.Texture;
using CUE4Parse.UE4.Assets.Exports.StaticMesh;
using CUE4Parse_Conversion.Animations;
using CUE4Parse_Conversion.Meshes;

class CounselorExporter(DefaultFileProvider provider, string outDir, Action<UObject> queue)
{
    public class Part
    {
        public string Name = "", File = "";
        /// "MasterPose" (shares the body's bones by name) or a body bone the part's root is attached to.
        public string Attach = "";
        /// Relative transform to the attach point, in Unreal space (cm).
        public Xform? Transform;
        public Dictionary<string, string>? MaterialMap;
    }

    public class Bone
    {
        public string Name = "";
        public int Parent;
        /// The skeleton's translation retargeting mode: "Skeleton" takes the translation from the mesh's own bind
        /// pose, "Animation" from the animation.
        public string Retarget = "";
    }

    public class AnimInfo
    {
        public string File = "";
        public float Length, Fps;
        public int Frames;
        public bool RootMotion;
        /// The root bone keeps its animated translation (paired kill animations, authored around a shared origin).
        public bool KeepRoot;
        /// The montage's notify states (e.g. AttackTraceNotify: when the weapon hits) and named notifies, seconds.
        public List<Window>? Windows;
    }

    /// A RichCurve key: Interp "linear", "constant" or "cubic".
    public class CurveKey
    {
        public float Time, Value, ArriveTangent, LeaveTangent;
        public string Interp = "";
    }

    public class Window
    {
        public string Name = "";
        public float Start, End;
    }

    public class Info
    {
        public string Name = "", Body = "";
        /// Capsule radius (cm).
        public float CapsuleRadius = 34;
        /// CharacterMesh0 relative to the capsule (Unreal space).
        public Xform? MeshTransform;
        public float CapsuleHalfHeight;
        public List<Part> Parts = new();
        /// The skin masks the game's alpha mask compiler (MM_AlphaMaskCompiler_Skin) multiplies: the counselor's own
        /// (AlphaMaskApplicationMaterial's FaceMask) and the outfit's SkinAlphaMask. The body's skin is clipped where
        /// saturate(1 - a.r * b.r) falls below its mask threshold (under the clothes).
        public List<string> SkinMasks = new();
        public List<Bone> Bones = new();
        public SortedDictionary<string, AnimInfo> Animations = new(StringComparer.OrdinalIgnoreCase);
        /// The skeleton's and mesh's sockets: name -> bone and offset (Unreal space).
        public SortedDictionary<string, Part> Sockets = new();
        /// Gameplay values: Blueprint overrides, else the C++ constructor's defaults.
        public SortedDictionary<string, float>? Values;
        /// Stat-modified values (FStatModifiedValue): [BaseValue, then StatModifiers by ECounselorStats].
        public SortedDictionary<string, float[]>? Stats;
        /// Float curves the values refer to, by property name.
        public SortedDictionary<string, List<CurveKey>>? Curves;
        /// Sounds by property name, as "SoundCue'Name'" / "SoundWave'Name'" (f13-export sounds exports them).
        public SortedDictionary<string, string>? Sounds;
        /// The character's music object (MusicObjectClass's defaults: MusicData, idle times, warning cues), object
        /// references as their names ("SoundCue'Name'").
        public Newtonsoft.Json.Linq.JObject? Music;
        /// The body mesh's physics asset (the ragdoll): one body per simulated bone, and the joints between them.
        public List<RagdollBody>? Ragdoll;
        public List<RagdollJoint>? Joints;
    }

    /// A physics asset body: its bone's capsules ("Sphyl": Radius, Length between the sphere centres, along the
    /// element's Z), spheres or boxes (Radius is half the smallest extent), relative to the bone, in cm.
    public class RagdollBody
    {
        public string Bone = "", Shape = "";
        public float Radius, Length;
        public Xform? Transform;
    }

    /// A physics constraint: Bone1 (the child) moves relative to Bone2 within the swing cone and twist limit
    /// (degrees; 0 locked, 180 free). The limits are measured between the joint's frames: Pri1/Sec1 (X and Y axes)
    /// in Bone1's space, Pri2/Sec2 in Bone2's (Unreal bone space). Swing1 turns X toward Y, Swing2 toward Z.
    public class RagdollJoint
    {
        public string Bone1 = "", Bone2 = "";
        public float Swing1, Swing2, Twist;
        public float[] Pri1 = [1, 0, 0], Sec1 = [0, 1, 0], Pri2 = [1, 0, 0], Sec2 = [0, 1, 0];
    }

    static Xform Xf(FVector t, FRotator r) => new()
    {
        T = [t.X, t.Y, t.Z],
        R = r.Quaternion() is var q ? [q.X, q.Y, q.Z, q.W] : [0, 0, 0, 1],
        S = [1, 1, 1],
    };

    static T? Prop<T>(UObject o, string name) => o.GetOrDefault<T?>(name);

    UObject Component(UObject cdo, string name) =>
        cdo.GetOrDefault<FPackageIndex>(name)?.Load() ?? throw new Exception($"{cdo.Name} has no {name}");

    /// Queues a texture for export; its path as materials.json writes texture paths (no extension).
    string Texture(UTexture t)
    {
        queue(t);
        return t.GetPathName().Split('.')[0].TrimStart('/');
    }

    static string MeshFile(USkeletalMesh m) => new CUE4Parse_Conversion.Exporters.SkinnedAssetExporter(m).SavePath + ".glb";

    public void Export(string blueprint, IEnumerable<string> animations, ICollection<string> keepRoot)
    {
        var cls = provider.LoadPackageObject<UBlueprintGeneratedClass>(blueprint + "." + blueprint.Split('/')[^1] + "_C");
        var cdo = cls.ClassDefaultObject.Load()!;
        var info = new Info { Name = blueprint.Split('/')[^1] };

        var meshComp = Component(cdo, "Mesh");
        var body = meshComp.GetOrDefault<FPackageIndex>("SkeletalMesh").Load<USkeletalMesh>()!;
        queue(body);
        info.Body = MeshFile(body);
        info.MeshTransform = Xf(meshComp.GetOrDefault<FVector>("RelativeLocation"), meshComp.GetOrDefault<FRotator>("RelativeRotation"));
        info.CapsuleHalfHeight = Component(cdo, "CapsuleComponent").GetOrDefault("CapsuleHalfHeight", 88f);

        // Default outfit: its clothing mesh, each slot's first material option.
        var outfitClass = cdo.GetOrDefault<FPackageIndex>("DefaultCounselorOutfit").Load<UBlueprintGeneratedClass>()!;
        var outfit = outfitClass.ClassDefaultObject.Load()!;
        var clothing = provider.LoadPackageObject<USkeletalMesh>(outfit.GetOrDefault<FSoftObjectPath>("ClothingMesh").AssetPathName.Text);
        queue(clothing);
        var map = new Dictionary<string, string>();
        foreach (var slot in outfit.GetOrDefault<FStructFallback[]>("ClothingSlots", []))
        {
            // Cooked meshes have no slot names (SlotMaterialName can't match); SlotIndex is the material index.
            var index = slot.GetOrDefault<int>("SlotIndex");
            var option = slot.GetOrDefault<FPackageIndex[]>("MaterialOptions", []).FirstOrDefault()?.Load<UBlueprintGeneratedClass>();
            var path = option?.ClassDefaultObject.Load()?.GetOrDefault<FSoftObjectPath>("MaterialInstance").AssetPathName.Text;
            if (string.IsNullOrEmpty(path) || path == "None") continue;
            var material = provider.LoadPackageObject<UMaterialInterface>(path);
            queue(material);
            if (index < clothing.SkeletalMaterials.Length && clothing.SkeletalMaterials[index].MaterialInterface?.Load() is UMaterialInterface own
                && own.Name != material.Name)
                map[own.Name] = material.Name;
        }
        info.Parts.Add(new Part { Name = "Outfit", File = MeshFile(clothing), Attach = "MasterPose", MaterialMap = map.Count > 0 ? map : null });
        var compiler = cdo.GetOrDefault<FSoftObjectPath>("AlphaMaskApplicationMaterial").AssetPathName.Text;
        if (!string.IsNullOrEmpty(compiler) && compiler != "None"
            && provider.LoadPackageObject<UMaterialInstanceConstant>(compiler).TextureParameterValues.FirstOrDefault(p => p.Name == "FaceMask")?.ParameterValue.Load<UTexture>() is { } faceMask)
            info.SkinMasks.Add(Texture(faceMask));
        var mask = outfit.GetOrDefault<FSoftObjectPath>("SkinAlphaMask").AssetPathName.Text;
        if (!string.IsNullOrEmpty(mask) && mask != "None")
            info.SkinMasks.Add(Texture(provider.LoadPackageObject<UTexture>(mask)));

        // The face hangs off the head (its body part component has no socket in the Blueprint; the skeleton's
        // headSocket is the head bone with no offset), the hair off the face.
        var face = Component(cdo, "FaceLimb");
        var faceMesh = face.GetOrDefault<FPackageIndex>("SkeletalMesh").Load<USkeletalMesh>()!;
        queue(faceMesh);
        info.Parts.Add(new Part { Name = "Face", File = MeshFile(faceMesh), Attach = "head", Transform = Xf(face.GetOrDefault<FVector>("RelativeLocation"), face.GetOrDefault<FRotator>("RelativeRotation")) });
        var hair = Component(cdo, "Hair");
        var hairMesh = hair.GetOrDefault<FPackageIndex>("SkeletalMesh").Load<USkeletalMesh>()!;
        queue(hairMesh);
        info.Parts.Add(new Part { Name = "Hair", File = MeshFile(hairMesh), Attach = "Face", Transform = Xf(hair.GetOrDefault<FVector>("RelativeLocation"), hair.GetOrDefault<FRotator>("RelativeRotation")) });

        var skeleton = body.Skeleton.Load<USkeleton>()!;
        var bones = skeleton.ReferenceSkeleton.FinalRefBoneInfo;
        for (var i = 0; i < bones.Length; i++)
            info.Bones.Add(new Bone { Name = bones[i].Name.Text, Parent = bones[i].ParentIndex, Retarget = skeleton.BoneTree[i].ToString() });

        // Noise, visibility and light (SCCounselorCharacter::GetNoiseLevel, SCCharacter::LineOfSight, the light
        // check of her Darkness fear event). Defaults: the counselor constructor (noise 1 crouching, 2 walking,
        // 3 running, 4 sprinting, 10 swimming; MinLightStrength 0.3) and SCCharacter's (MaxVisibilityDistance 6000).
        var values = new SortedDictionary<string, float>();
        foreach (var (name, fallback) in new[] { ("CrouchingNoiseMod", 1f), ("WalkingNoiseMod", 2f), ("RunningNoiseMod", 3f),
                     ("SprintingNoiseMod", 4f), ("SwimmingNoiseMod", 10f), ("MaxVisibilityDistance", 6000f),
                     ("MinLightStrength", 0.3f) })
            values[name] = ClassValue<float?>(cls, name) ?? fallback;
        // Her flashlight (the Flashlight spot light; defaults: Unreal's spot light) and how fast it turns to its aim
        // (FlashlightLerpSpeed, constructor 15).
        var flash = Component(cdo, "Flashlight");
        var color = flash.GetOrDefault<FColor?>("LightColor") ?? new FColor(255, 255, 255, 255);
        values["FlashlightIntensity"] = flash.GetOrDefault("Intensity", 5000f);
        values["FlashlightColorR"] = color.R;
        values["FlashlightColorG"] = color.G;
        values["FlashlightColorB"] = color.B;
        values["FlashlightTemperature"] = flash.GetOrDefault("bUseTemperature", false) ? flash.GetOrDefault("Temperature", 6500f) : 0f;
        values["FlashlightInnerConeAngle"] = flash.GetOrDefault("InnerConeAngle", 0f);
        values["FlashlightOuterConeAngle"] = flash.GetOrDefault("OuterConeAngle", 44f);
        values["FlashlightAttenuationRadius"] = flash.GetOrDefault("AttenuationRadius", 1000f);
        values["FlashlightFalloffExponent"] = flash.GetOrDefault("LightFalloffExponent", 8f);
        values["FlashlightInverseSquared"] = flash.GetOrDefault("bUseInverseSquaredFalloff", true) ? 1f : 0f;
        values["FlashlightLerpSpeed"] = ClassValue<float?>(cls, "FlashlightLerpSpeed") ?? 15f;
        info.Values = values;
        info.Stats = new() { ["NoiseStatMod"] = StatValue(cls, "NoiseStatMod", 1f) };
        // Holding her breath while hiding (constructor: MaxBreath 100, MinHoldBreathThreshold 50, MinHoldBreathTimer
        // 1.5; BreathHoldModifier 10 with Stealth 1, BreathRechargeModifier 10 with Composure 1, BreathNoiseMod 1 with
        // Stealth 0.2).
        foreach (var (name, fallback) in new[] { ("MaxBreath", 100f), ("MinHoldBreathThreshold", 50f), ("MinHoldBreathTimer", 1.5f) })
            values[name] = ClassValue<float?>(cls, name) ?? fallback;
        info.Stats["BreathHoldModifier"] = StatValue(cls, "BreathHoldModifier", 10f, (6, 1f));
        info.Stats["BreathRechargeModifier"] = StatValue(cls, "BreathRechargeModifier", 10f, (5, 1f));
        info.Stats["BreathNoiseMod"] = StatValue(cls, "BreathNoiseMod", 1f, (6, 0.2f));
        info.Music = MusicObject(cls);
        // Her voice lines (VoiceOverObjectClass's VoiceOverData), as Sounds "VO:<name>".
        info.Sounds = VoiceOvers(cls);

        AddSockets(info, body, skeleton);
        AddRagdoll(info, body);
        WriteAnimations(info, skeleton, animations, "anims", keepRoot);
        Save(info, "counselors");
    }

    void WriteAnimations(Info info, USkeleton skeleton, IEnumerable<string> animations, string dir, ICollection<string> keepRoot)
    {
        Directory.CreateDirectory(Path.Combine(outDir, dir));
        foreach (var path in animations)
        {
            var windows = Windows(path);
            foreach (var seq in Sequences(path))
            {
                if (info.Animations.ContainsKey(seq.Name)) continue;
                var a = WriteSequence(seq, skeleton, dir, keepRoot.Contains(path));
                a.Windows = windows;
                info.Animations[seq.Name] = a;
            }
        }
    }

    /// The body mesh's PhysicsAsset: bodies (the largest shape of each) and constraints.
    static void AddRagdoll(Info info, USkeletalMesh body)
    {
        if (body.GetOrDefault<FPackageIndex>("PhysicsAsset")?.Load() is not { } asset) return;
        info.Ragdoll = new();
        info.Joints = new();
        foreach (var setup in asset.GetOrDefault<FPackageIndex[]>("SkeletalBodySetups", []).Select(b => b.Load()))
        {
            if (setup == null) continue;
            var geom = setup.GetOrDefault<FStructFallback>("AggGeom");
            var bone = setup.GetOrDefault<FName>("BoneName").Text;
            RagdollBody? best = null;
            void Consider(RagdollBody b) { if (best == null || b.Radius + b.Length / 2 > best.Radius + best.Length / 2) best = b; }
            foreach (var e in geom?.GetOrDefault<FStructFallback[]>("SphylElems", []) ?? [])
                Consider(new RagdollBody { Bone = bone, Shape = "Sphyl", Radius = e.GetOrDefault<float>("Radius"), Length = e.GetOrDefault<float>("Length"),
                    Transform = Xf(e.GetOrDefault<FVector>("Center"), e.GetOrDefault<FRotator>("Rotation")) });
            foreach (var e in geom?.GetOrDefault<FStructFallback[]>("SphereElems", []) ?? [])
                Consider(new RagdollBody { Bone = bone, Shape = "Sphere", Radius = e.GetOrDefault<float>("Radius"),
                    Transform = Xf(e.GetOrDefault<FVector>("Center"), new FRotator()) });
            foreach (var e in geom?.GetOrDefault<FStructFallback[]>("BoxElems", []) ?? [])
            {
                float x = e.GetOrDefault<float>("X"), y = e.GetOrDefault<float>("Y"), z = e.GetOrDefault<float>("Z");
                Consider(new RagdollBody { Bone = bone, Shape = "Box", Radius = Math.Min(x, Math.Min(y, z)) / 2, Length = Math.Max(x, Math.Max(y, z)) - Math.Min(x, Math.Min(y, z)),
                    Transform = Xf(e.GetOrDefault<FVector>("Center"), e.GetOrDefault<FRotator>("Rotation")) });
            }
            if (best != null) info.Ragdoll.Add(best);
        }
        static float Limit(FStructFallback? limit, string motion, string degrees)
        {
            // FConeConstraint/FTwistConstraint defaults: 45 degrees, ACM_Free.
            var m = limit?.GetOrDefault<FName>(motion).Text ?? "";
            if (m == "None" || m == "") m = limit?.GetOrDefault<string>(motion) ?? "ACM_Free";
            if (m.EndsWith("Locked")) return 0;
            if (m.EndsWith("Limited")) return limit?.GetOrDefault(degrees, 45f) ?? 45f;
            return 180;
        }
        // FConstraintInstance's axes default to X and Y.
        static float[] Axis(FStructFallback inst, string name, float[] fallback) =>
            inst.TryGetValue(out FVector v, name) ? [v.X, v.Y, v.Z] : fallback;
        foreach (var c in asset.GetOrDefault<FPackageIndex[]>("ConstraintSetup", []).Select(c => c.Load()))
        {
            var inst = c?.GetOrDefault<FStructFallback>("DefaultInstance");
            if (inst == null) continue;
            var profile = inst.GetOrDefault<FStructFallback>("ProfileInstance");
            var cone = profile?.GetOrDefault<FStructFallback>("ConeLimit");
            var twist = profile?.GetOrDefault<FStructFallback>("TwistLimit");
            info.Joints.Add(new RagdollJoint
            {
                Bone1 = inst.GetOrDefault<FName>("ConstraintBone1").Text,
                Bone2 = inst.GetOrDefault<FName>("ConstraintBone2").Text,
                Swing1 = Limit(cone, "Swing1Motion", "Swing1LimitDegrees"),
                Swing2 = Limit(cone, "Swing2Motion", "Swing2LimitDegrees"),
                Twist = Limit(twist, "TwistMotion", "TwistLimitDegrees"),
                Pri1 = Axis(inst, "PriAxis1", [1, 0, 0]),
                Sec1 = Axis(inst, "SecAxis1", [0, 1, 0]),
                Pri2 = Axis(inst, "PriAxis2", [1, 0, 0]),
                Sec2 = Axis(inst, "SecAxis2", [0, 1, 0]),
            });
        }
    }

    /// Every socket of the body mesh and its skeleton (the mesh's win), as Parts without a file.
    static void AddSockets(Info info, USkeletalMesh body, USkeleton skeleton)
    {
        foreach (var s in skeleton.GetOrDefault<FPackageIndex[]>("Sockets", []).Concat(body.GetOrDefault<FPackageIndex[]>("Sockets", [])).Select(s => s.Load()))
        {
            if (s == null) continue;
            info.Sockets[s.GetOrDefault<FName>("SocketName").Text] = new Part
            {
                Name = s.GetOrDefault<FName>("SocketName").Text,
                Attach = s.GetOrDefault<FName>("BoneName").Text,
                Transform = Xf(s.GetOrDefault<FVector>("RelativeLocation"), s.GetOrDefault<FRotator>("RelativeRotation")),
            };
        }
    }

    void Save(Info info, string dir)
    {
        Directory.CreateDirectory(Path.Combine(outDir, dir));
        File.WriteAllText(Path.Combine(outDir, dir, info.Name + ".json"),
            Newtonsoft.Json.JsonConvert.SerializeObject(info, Newtonsoft.Json.Formatting.Indented));
        Console.WriteLine($"{info.Name}: {info.Parts.Count} parts, {info.Animations.Count} animations");
    }

    /// A component property, looked up through the component's archetypes (a subclass only stores what it overrides).
    static T? Inherited<T>(UObject component, string name)
    {
        for (UObject? a = component; a != null; a = a.Template?.Load())
            if (a.TryGetValue(out T v, name)) return v;
        return default;
    }

    /// The killer's character: body (CharacterMesh0), mask and hair (same skeleton: master pose), the weapon in its hand
    /// socket, and the animations f13-game plays on it.
    public void ExportKiller(string blueprint, IEnumerable<string> animations, ICollection<string> keepRoot)
    {
        var cls = provider.LoadPackageObject<UBlueprintGeneratedClass>(blueprint + "." + blueprint.Split('/')[^1] + "_C");
        var cdo = cls.ClassDefaultObject.Load()!;
        var info = new Info { Name = blueprint.Split('/')[^1] };

        var meshComp = Component(cdo, "Mesh");
        var body = Inherited<FPackageIndex>(meshComp, "SkeletalMesh")!.Load<USkeletalMesh>()!;
        queue(body);
        info.Body = MeshFile(body);
        info.MeshTransform = Xf(Inherited<FVector>(meshComp, "RelativeLocation"), Inherited<FRotator>(meshComp, "RelativeRotation"));
        var capsule = Component(cdo, "CapsuleComponent");
        // Not set by the killer's Blueprints or its C++ constructor: ACharacter's InitCapsuleSize(34, 88).
        info.CapsuleHalfHeight = Inherited<float?>(capsule, "CapsuleHalfHeight") ?? 88f;
        info.CapsuleRadius = Inherited<float?>(capsule, "CapsuleRadius") ?? 34f;

        void MasterPose(string name, UObject comp)
        {
            if (Inherited<FPackageIndex>(comp, "SkeletalMesh")?.Load<USkeletalMesh>() is not { } mesh) return;
            // Jason_Base's separate hair component is hidden (bVisible false) unless a Jason turns it on.
            if (Inherited<bool?>(comp, "bVisible") == false) return;
            queue(mesh);
            info.Parts.Add(new Part { Name = name, File = MeshFile(mesh), Attach = "MasterPose", MaterialMap = Overrides(comp, mesh) });
        }
        MasterPose("Mask", Component(cdo, "JasonSkeletalMask"));
        // The hair is a Blueprint-added component: its template (overridden per Jason) is a _GEN_VARIABLE export.
        var hair = provider.LoadPackage(blueprint).GetExports().FirstOrDefault(e => e.Name == "Jason_Hair_GEN_VARIABLE");
        if (hair != null) MasterPose("Hair", hair);

        // The weapon hangs off its HandAttachSocket (a skeleton socket: bone + offset).
        var weaponPath = cdo.GetOrDefault<FSoftObjectPath>("WeaponClass").AssetPathName.Text;
        var weaponClass = provider.LoadPackageObject<UBlueprintGeneratedClass>(weaponPath);
        var weapon = weaponClass.ClassDefaultObject.Load()!;
        var socketName = weapon.GetOrDefault<FName>("HandAttachSocket").Text;
        var skeleton = body.Skeleton.Load<USkeleton>()!;
        var socket = body.GetOrDefault<FPackageIndex[]>("Sockets", []).Concat(skeleton.GetOrDefault<FPackageIndex[]>("Sockets", []))
            .Select(s => s.Load()).FirstOrDefault(s => s?.GetOrDefault<FName>("SocketName").Text == socketName)
            ?? throw new Exception($"no socket {socketName} on {body.Name}");
        info.Parts.Add(new Part
        {
            Name = "WeaponSocket",
            Attach = socket.GetOrDefault<FName>("BoneName").Text,
            Transform = Xf(socket.GetOrDefault<FVector>("RelativeLocation"), socket.GetOrDefault<FRotator>("RelativeRotation")),
        });
        var item = Component(weapon, "ItemMesh3P");
        var weaponMesh = Inherited<FPackageIndex>(item, "StaticMesh")!.Load<UStaticMesh>()!;
        queue(weaponMesh);
        info.Parts.Add(new Part
        {
            Name = "Weapon",
            File = new CUE4Parse_Conversion.Exporters.StaticMeshExporter(weaponMesh).SavePath + ".glb",
            Attach = "WeaponSocket",
            Transform = Xf(Inherited<FVector>(item, "RelativeLocation"), Inherited<FRotator>(item, "RelativeRotation")),
        });

        // Gameplay values. Defaults: SCKillerCharacter's constructor (AttackDelay 1, grab 100..200 cm within 45 deg,
        // attack interact 200 cm / 45 deg, DamageCharacterModifier and DamageWorldModifier 1) and SCCharacterMovement's / UE 4.18's
        // (MaxAcceleration 2048, RotationRate 360).
        var values = new SortedDictionary<string, float>();
        foreach (var (name, fallback) in new[] { ("AttackDelay", 1f), ("MaxGrabDistance", 200f), ("MinGrabDistance", 100f), ("MinGrabAngle", 45f),
                     ("MaxAttackInteractCounselorDistance", 200f), ("MaxAttackInteractAngle", 45f), ("DamageCharacterModifier", 1f),
                     ("DamageWorldModifier", 1f) })
            values[name] = ClassValue<float?>(cls, name) ?? fallback;
        var move = Component(cdo, "CharacterMovement");
        foreach (var (name, fallback) in new[] { ("MaxWalkSpeed", 104f), ("MaxRunSpeed", 280f), ("MaxSprintSpeed", 450f), ("MaxCombatSpeed", 104f), ("MaxAcceleration", 2048f) })
            values[name] = Inherited<float?>(move, name) ?? fallback;
        values["RotationRateYaw"] = Inherited<FRotator?>(move, "RotationRate")?.Yaw ?? 360f;
        values["CanRun"] = (Inherited<FPackageIndex>(meshComp, "AnimClass")?.Load<UStruct>() is { } animClass ? ClassValue<bool?>(animClass, "bCanRun") : null) == true ? 1 : 0;
        var stats = ClassValue<FStructFallback>(weaponClass, "WeaponStats");
        values["WeaponDamage"] = stats?.GetOrDefault<float>("Damage") ?? 0;
        // What he hears and senses: sound blips (constructor: MinBlipRadius 1000, NoiseLevelThreshold 2,
        // NoiseDistanceThreshold 50000, blips every 0.5..2 s), Sense (RageUnlockTime 900; SenseDuration and
        // RagedSenseDistanceModifier have no constructor value).
        foreach (var (name, fallback) in new[] { ("MinBlipRadius", 1000f), ("NoiseLevelThreshold", 2f), ("NoiseDistanceThreshold", 50000f),
                     ("MinBlipFrequencyTime", 0.5f), ("MaxBlipFrequencyTime", 2f), ("SenseDuration", 0f), ("RagedSenseDistanceModifier", 1f),
                     ("RageUnlockTime", 900f) })
            values[name] = ClassValue<float?>(cls, name) ?? fallback;
        // Shift, Stalk and Morph (constructor: StalkActiveTime 20, IdleStalkModifier 0.1, TeleportScreenTime 1;
        // ShiftSpeed and ShiftTime have no constructor value).
        foreach (var (name, fallback) in new[] { ("ShiftSpeed", 0f), ("ShiftTime", 0f), ("StalkActiveTime", 20f), ("IdleStalkModifier", 0.1f),
                     ("TeleportScreenTime", 1f) })
            values[name] = ClassValue<float?>(cls, name) ?? fallback;
        // His kit (constructor: NumKnives none, InitialTrapCount 5, TrapPlacementDistance 75, ThrowingKnifeCooldown
        // 3.5); the knife's WeaponStats.Damage.
        foreach (var (name, fallback) in new[] { ("NumKnives", 0f), ("InitialTrapCount", 5f), ("TrapPlacementDistance", 75f), ("ThrowingKnifeCooldown", 3.5f) })
            values[name] = ClassValue<object>(cls, name) switch { int i => i, float f => f, _ => fallback };
        if (ClassValue<FPackageIndex>(cls, "ThrowingKnifeClass")?.Load<UStruct>() is { } knife)
            values["ThrowingKnifeDamage"] = ClassValue<FStructFallback>(knife, "WeaponStats")?.GetOrDefault<float>("Damage") ?? 0;
        // The music counselors hear near him (NearCounselorMusic: full volume within NearCounselorMusicMaxVolumeDistance,
        // fading out over NearCounselorMusicDistance beyond it; constructor 300 and 4000).
        foreach (var (name, fallback) in new[] { ("NearCounselorMusicDistance", 4000f), ("NearCounselorMusicMaxVolumeDistance", 300f) })
            values[name] = ClassValue<float?>(cls, name) ?? fallback;
        info.Sounds = new();
        if (ClassValue<FPackageIndex>(cls, "NearCounselorMusic")?.Load<UObject>() is { } near)
            info.Sounds["NearCounselorMusic"] = $"{near.ExportType}'{near.Name}'";
        // The sound blip actor's lifetime (InitialLifeSpan).
        if (ClassValue<FPackageIndex>(cls, "SoundBlipClass")?.Load<UStruct>() is { } blip)
            values["SoundBlipLifeSpan"] = ClassValue<float?>(blip, "InitialLifeSpan") ?? 0;
        info.Values = values;
        info.Curves = new();
        foreach (var name in new[] { "SenseFearRadiusCurve", "SenseFearHidingRadiusCurve", "RageSenseDistanceCurve" })
            if (CurveValue(cls, name) is { } keys) info.Curves[name] = keys;

        var bones = skeleton.ReferenceSkeleton.FinalRefBoneInfo;
        for (var i = 0; i < bones.Length; i++)
            info.Bones.Add(new Bone { Name = bones[i].Name.Text, Parent = bones[i].ParentIndex, Retarget = skeleton.BoneTree[i].ToString() });
        AddSockets(info, body, skeleton);
        WriteAnimations(info, skeleton, animations, "anims/jason", keepRoot);
        Save(info, "killers");
    }

    /// A component's OverrideMaterials, as the mesh's own material name -> replacement.
    Dictionary<string, string>? Overrides(UObject comp, USkeletalMesh mesh)
    {
        var map = new Dictionary<string, string>();
        var overrides = Inherited<FPackageIndex[]>(comp, "OverrideMaterials") ?? [];
        for (var i = 0; i < overrides.Length && i < mesh.SkeletalMaterials.Length; i++)
        {
            if (overrides[i].Load<UMaterialInterface>() is not { } m) continue;
            queue(m);
            if (mesh.SkeletalMaterials[i].MaterialInterface?.Load() is UMaterialInterface own && own.Name != m.Name) map[own.Name] = m.Name;
        }
        return map.Count > 0 ? map : null;
    }

    /// A montage's notify states (those with a duration) and its named notifies (no notify object: events for the
    /// anim Blueprint, e.g. "shouldRagdoll" in the counselors' kill montages; zero length), by name.
    List<Window>? Windows(string path)
    {
        var obj = provider.LoadPackageObject(path + "." + path.Split('/')[^1]);
        var list = obj.GetOrDefault<FStructFallback[]>("Notifies", [])
            .Select(n => (Name: n.GetOrDefault<FName>("NotifyName").Text, Start: n.GetOrDefault<float>("LinkValue"), Duration: n.GetOrDefault<float>("Duration"),
                Named: n.GetOrDefault<FPackageIndex>("Notify") is not { IsNull: false } && n.GetOrDefault<FPackageIndex>("NotifyStateClass") is not { IsNull: false }))
            .Where(n => n.Duration > 0 || n.Named)
            .Select(n => new Window { Name = n.Name, Start = n.Start, End = n.Start + n.Duration })
            .ToList();
        return list.Count > 0 ? list : null;
    }

    /// A stat-modified value through the Blueprint's parents: [BaseValue, StatModifiers[0..7]] (StatModifiers is a
    /// static array indexed by stat; unset entries are 0). `baseValue` is the C++ constructor's.
    static float[] StatValue(UStruct cls, string name, float baseValue, params (int Stat, float Value)[] ctorModifiers)
    {
        var v = new float[8];
        v[0] = baseValue;
        foreach (var (stat, value) in ctorModifiers) v[1 + stat] = value;
        if (ClassValue<FStructFallback>(cls, name) is not { } s) return v;
        foreach (var p in s.Properties)
        {
            if (p.Name.Text == "BaseValue") v[0] = p.Tag?.GetValue<float>() ?? v[0];
            else if (p.Name.Text == "StatModifiers" && p.ArrayIndex < 7) v[1 + p.ArrayIndex] = p.Tag?.GetValue<float>() ?? 0;
        }
        return v;
    }

    /// A curve property's keys through the Blueprint's parents (null when unset).
    static List<CurveKey>? CurveValue(UStruct cls, string name)
    {
        if (ClassValue<FPackageIndex>(cls, name)?.Load() is not { } curve) return null;
        var rich = curve.GetOrDefault<FStructFallback>("FloatCurve");
        var keys = rich?.GetOrDefault<FRichCurveKey[]>("Keys", []) ?? [];
        return keys.Select(k => new CurveKey
        {
            Time = k.Time, Value = k.Value, ArriveTangent = k.ArriveTangent, LeaveTangent = k.LeaveTangent,
            Interp = k.InterpMode switch { ERichCurveInterpMode.RCIM_Linear => "linear", ERichCurveInterpMode.RCIM_Constant => "constant", _ => "cubic" },
        }).ToList();
    }

    /// A class default, through the Blueprint's parents (each CDO only stores what its class overrides).
    /// A character's voice lines (VoiceOverObjectClass's VoiceOverData: name -> sound), as Sounds "VO:<name>".
    static SortedDictionary<string, string> VoiceOvers(UStruct? cls)
    {
        var result = new SortedDictionary<string, string>();
        if (ClassValue<FPackageIndex>(cls, "VoiceOverObjectClass")?.Load<UStruct>() is not UBlueprintGeneratedClass bp || bp.ClassDefaultObject.Load() is not { } cdo)
            return result;
        var props = Newtonsoft.Json.Linq.JObject.Parse(Newtonsoft.Json.JsonConvert.SerializeObject(cdo))["Properties"];
        foreach (var d in props?["VoiceOverData"] as Newtonsoft.Json.Linq.JArray ?? [])
        {
            var name = d["VoiceOverName"]?.ToString();
            var path = d["VoiceOverFile"]?["AssetPathName"]?.ToString();
            if (string.IsNullOrEmpty(name) || string.IsNullOrEmpty(path) || path == "None") continue;
            result["VO:" + name] = $"SoundCue'{path.Split('.')[^1]}'";
        }
        return result;
    }

    /// The defaults of a character's MusicObjectClass, with object references flattened to their names.
    static Newtonsoft.Json.Linq.JObject? MusicObject(UStruct? cls)
    {
        if (ClassValue<FPackageIndex>(cls, "MusicObjectClass")?.Load<UStruct>() is not UBlueprintGeneratedClass bp || bp.ClassDefaultObject.Load() is not { } cdo)
            return null;
        static Newtonsoft.Json.Linq.JToken Flatten(Newtonsoft.Json.Linq.JToken t) => t switch
        {
            Newtonsoft.Json.Linq.JObject o when o["ObjectName"] is { } n => n.DeepClone(),
            Newtonsoft.Json.Linq.JObject o => new Newtonsoft.Json.Linq.JObject(o.Properties().Select(p => new Newtonsoft.Json.Linq.JProperty(p.Name, Flatten(p.Value)))),
            Newtonsoft.Json.Linq.JArray a => new Newtonsoft.Json.Linq.JArray(a.Select(Flatten)),
            _ => t.DeepClone(),
        };
        var props = Newtonsoft.Json.Linq.JObject.Parse(Newtonsoft.Json.JsonConvert.SerializeObject(cdo))["Properties"];
        return props == null ? null : Flatten(props) as Newtonsoft.Json.Linq.JObject;
    }

    static T? ClassValue<T>(UStruct? cls, string name)
    {
        for (; cls != null; cls = cls.SuperStruct?.Load<UStruct>())
            if (cls is UBlueprintGeneratedClass bp && bp.ClassDefaultObject.Load() is { } cdo && cdo.TryGetValue(out T v, name)) return v;
        return default;
    }

    /// A sequence, or each sequence a montage plays.
    IEnumerable<UAnimSequence> Sequences(string path)
    {
        var obj = provider.LoadPackageObject(path + "." + path.Split('/')[^1]);
        if (obj is UAnimSequence s) { yield return s; yield break; }
        if (obj is UAnimMontage m)
            foreach (var slot in m.SlotAnimTracks ?? [])
            foreach (var seg in slot.AnimTrack.AnimSegments ?? [])
                if (seg.AnimReference.TryLoad(out UAnimSequence? seq) && seq != null) yield return seq;
    }

    // .anim, little-endian: "F13A", u32 version 1, f32 fps, u32 frames, u32 track count, then per track:
    // u16 name length + UTF-8 bone name, u8 flags (1 rotation, 2 translation), frames x f32[4] rotation (Unreal
    // x, y, z, w) if flagged, frames x f32[3] translation (Unreal cm) if flagged. Bone-local transforms. Translation is
    // only written for the skeleton's "Animation" retargeting bones (the others keep the mesh's bind pose, as Unreal
    // retargets them). Root motion and bForceRootLock sequences have the root locked at its reference pose (Unreal's
    // default RootMotionRootLock::RefPose), since the game moves the character instead.
    AnimInfo WriteSequence(UAnimSequence seq, USkeleton skeleton, string dir, bool keepRoot)
    {
        var set = seq.ConvertAnims();
        var anim = set.Sequences[0];
        var frames = Math.Max(anim.NumFrames, 1);
        var fps = anim.FramesPerSecond > 0 ? anim.FramesPerSecond : 30f;
        var rootMotion = seq.GetOrDefault<bool>("bEnableRootMotion");
        // Root motion sequences, and in-place cycles authored moving forward (bForceRootLock: the locomotion cycles),
        // keep the root at its reference pose.
        var rootLock = !keepRoot && (rootMotion || seq.GetOrDefault<bool>("bForceRootLock"));
        var refPose = skeleton.ReferenceSkeleton.FinalRefBonePose;
        var file = Path.Combine(dir, seq.Name + ".anim");
        using var w = new BinaryWriter(File.Create(Path.Combine(outDir, file)));
        w.Write("F13A"u8);
        w.Write(1u);
        w.Write(fps);
        w.Write((uint)frames);
        var tracks = Enumerable.Range(0, Math.Min(anim.Tracks.Count, skeleton.ReferenceSkeleton.FinalRefBoneInfo.Length))
            .Where(i => anim.Tracks[i].HasKeys()).ToList();
        w.Write((uint)tracks.Count);
        foreach (var i in tracks)
        {
            var track = anim.Tracks[i];
            var name = System.Text.Encoding.UTF8.GetBytes(skeleton.ReferenceSkeleton.FinalRefBoneInfo[i].Name.Text);
            w.Write((ushort)name.Length);
            w.Write(name);
            var translate = skeleton.BoneTree[i] == EBoneTranslationRetargetingMode.Animation;
            w.Write((byte)(translate ? 3 : 1));
            var rot = new FQuat[frames];
            var pos = new FVector[frames];
            for (var f = 0; f < frames; f++)
            {
                FQuat q = FQuat.Identity; FVector p = FVector.ZeroVector, s = FVector.OneVector;
                track.GetBoneTransform(f, frames, ref q, ref p, ref s);
                if (rootLock && i == 0) { q = refPose[0].Rotation; p = refPose[0].Translation; }
                rot[f] = q; pos[f] = p;
            }
            foreach (var q in rot) { w.Write(q.X); w.Write(q.Y); w.Write(q.Z); w.Write(q.W); }
            if (translate) foreach (var p in pos) { w.Write(p.X); w.Write(p.Y); w.Write(p.Z); }
        }
        return new AnimInfo { File = file.Replace('\\', '/'), Length = (frames - 1) / fps, Fps = fps, Frames = frames, RootMotion = rootMotion, KeepRoot = keepRoot };
    }
}
