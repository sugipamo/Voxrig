// Movement oracle for Java 1.21.11. Written against Mojang's published names,
// compiled against a locally remapped copy of the official server, then remapped
// back so it runs inside the unchanged official server JAR (see run.py).
// No game code is copied: the player below calls the official movement code
// and only ports the client-side input handling of LocalPlayer, noted inline.
package voxrig.oracle;

import com.google.gson.*;
import com.mojang.authlib.GameProfile;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import net.minecraft.tags.BlockTags;
import net.minecraft.tags.FluidTags;
import net.minecraft.world.level.material.FlowingFluid;
import net.minecraft.world.level.material.FluidState;
import net.minecraft.world.level.EmptyBlockGetter;
import net.minecraft.world.level.block.FenceGateBlock;
import net.minecraft.world.level.block.TrapDoorBlock;
import net.minecraft.world.phys.shapes.CollisionContext;
import net.minecraft.world.phys.shapes.VoxelShape;
import java.nio.file.*;
import java.util.*;
import java.util.concurrent.atomic.AtomicReference;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.resources.Identifier;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.util.Mth;
import net.minecraft.world.effect.MobEffect;
import net.minecraft.world.effect.MobEffectInstance;
import net.minecraft.world.entity.Pose;
import net.minecraft.world.entity.ai.attributes.Attribute;
import net.minecraft.world.entity.player.Abilities;
import net.minecraft.world.entity.player.Input;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.properties.Property;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.Vec2;
import net.minecraft.world.phys.Vec3;

public final class MovementOracle {
    /** A survival player driven like LocalPlayer, inside a server level. */
    static final class OraclePlayer extends Player {
        Input keys = Input.EMPTY;
        Input next = Input.EMPTY;
        Vec2 moveVector = Vec2.ZERO;
        boolean crouching;

        OraclePlayer(Level level) {
            super(level, new GameProfile(new UUID(0, 1), "oracle"));
            getAbilities().invulnerable = true;
        }

        @Override public GameType gameMode() { return GameType.SURVIVAL; }
        // LocalPlayer: the local instance is authoritative for its own movement.
        @Override public boolean isLocalPlayer() { return true; }
        @Override public boolean isClientAuthoritative() { return false; }
        @Override public boolean isShiftKeyDown() { return keys.shift(); }
        @Override public boolean isCrouching() { return crouching; }
        @Override public boolean isUnderWater() { return wasUnderwater; }
        boolean isMovingSlowly() { return isCrouching() || isVisuallyCrawling(); }

        static float impulse(boolean positive, boolean negative) {
            return positive == negative ? 0.0F : positive ? 1.0F : -1.0F;
        }

        // Port of the movement-relevant part of LocalPlayer.aiStep. Sprinting is
        // requested only by the sprint key (no double-tap), flying is not used.
        @Override public void aiStep() {
            Abilities abilities = getAbilities();
            crouching = !abilities.flying && !isSwimming() && !isPassenger()
                && canPlayerFitWithinBlocksAndEntitiesWhen(Pose.CROUCHING)
                && (isShiftKeyDown() || !isSleeping() && !canPlayerFitWithinBlocksAndEntitiesWhen(Pose.STANDING));
            keys = next;
            moveVector = new Vec2(impulse(keys.left(), keys.right()), impulse(keys.forward(), keys.backward())).normalized();
            if (!noPhysics) {
                double w = getBbWidth() * 0.35;
                moveTowardsClosestSpace(getX() - w, getZ() + w);
                moveTowardsClosestSpace(getX() - w, getZ() - w);
                moveTowardsClosestSpace(getX() + w, getZ() - w);
                moveTowardsClosestSpace(getX() + w, getZ() + w);
            }
            if (canStartSprinting() && keys.sprint()) setSprinting(true);
            if (isSprinting()) {
                if (isSwimming()) {
                    if (shouldStopSwimSprinting()) setSprinting(false);
                } else if (shouldStopRunSprinting()) {
                    setSprinting(false);
                }
            }
            if (isInWater() && keys.shift() && isAffectedByFluids()) goDownInWater();
            super.aiStep();
        }

        boolean hasForwardImpulse() { return moveVector.y > 1.0E-5F; }
        boolean isSprintingPossible(boolean flying) {
            return !isMobilityRestricted() && hasEnoughFoodToDoExhaustiveManoeuvres() && (flying || !isInShallowWater());
        }
        boolean canStartSprinting() {
            return !isSprinting() && hasForwardImpulse() && isSprintingPossible(getAbilities().flying)
                && !isSlowDueToUsingItem() && (!isFallFlying() || isUnderWater()) && (!isMovingSlowly() || isUnderWater());
        }
        net.minecraft.world.item.component.UseEffects useEffects() {
            return getUseItem().getOrDefault(net.minecraft.core.component.DataComponents.USE_EFFECTS,
                net.minecraft.world.item.component.UseEffects.DEFAULT);
        }
        boolean isSlowDueToUsingItem() { return isUsingItem() && !useEffects().canSprint(); }
        boolean shouldStopRunSprinting() {
            return !isSprintingPossible(getAbilities().flying) || !hasForwardImpulse()
                || horizontalCollision && !minorHorizontalCollision;
        }
        boolean shouldStopSwimSprinting() {
            return !isSprintingPossible(true) || !isInWater() || !hasForwardImpulse() && !onGround() && !keys.shift();
        }

        void moveTowardsClosestSpace(double x, double z) {
            BlockPos pos = BlockPos.containing(x, getY(), z);
            if (!suffocatesAt(pos)) return;
            double dx = x - pos.getX(), dz = z - pos.getZ();
            Direction best = null;
            double nearest = Double.MAX_VALUE;
            for (Direction d : new Direction[]{Direction.WEST, Direction.EAST, Direction.NORTH, Direction.SOUTH}) {
                double along = d.getAxis().choose(dx, 0.0, dz);
                double distance = d.getAxisDirection() == Direction.AxisDirection.POSITIVE ? 1.0 - along : along;
                if (distance < nearest && !suffocatesAt(pos.relative(d))) { nearest = distance; best = d; }
            }
            if (best != null) {
                Vec3 v = getDeltaMovement();
                if (best.getAxis() == Direction.Axis.X) setDeltaMovement(0.1 * best.getStepX(), v.y, v.z);
                else setDeltaMovement(v.x, v.y, 0.1 * best.getStepZ());
            }
        }
        boolean suffocatesAt(BlockPos pos) {
            AABB box = getBoundingBox();
            AABB column = new AABB(pos.getX(), box.minY, pos.getZ(), pos.getX() + 1.0, box.maxY, pos.getZ() + 1.0).deflate(1.0E-7);
            return level().collidesWithSuffocatingBlock(this, column);
        }

        // LocalPlayer.applyInput / modifyInput: the client scales the key vector.
        @Override public void applyInput() {
            Vec2 v = modifyInput(moveVector);
            xxa = v.x;
            zza = v.y;
            jumping = keys.jump();
        }
        Vec2 modifyInput(Vec2 v) {
            if (v.lengthSquared() == 0.0F) return v;
            Vec2 scaled = v.scale(0.98F);
            if (isUsingItem() && !isPassenger()) scaled = scaled.scale(useEffects().speedMultiplier());
            if (isMovingSlowly()) scaled = scaled.scale((float) getAttributeValue(net.minecraft.world.entity.ai.attributes.Attributes.SNEAKING_SPEED));
            float length = scaled.length();
            if (length <= 0.0F) return scaled;
            Vec2 unit = scaled.scale(1.0F / length);
            float ax = Math.abs(unit.x), ay = Math.abs(unit.y);
            float ratio = ay > ax ? ax / ay : ay / ax;
            return unit.scale(Math.min(length * Mth.sqrt(1.0F + Mth.square(ratio)), 1.0F));
        }

        // LocalPlayer.isHorizontalCollisionMinor (Entity's default is false).
        @Override protected boolean isHorizontalCollisionMinor(Vec3 movement) {
            float yaw = getYRot() * (float) (Math.PI / 180.0);
            double s = Mth.sin(yaw), c = Mth.cos(yaw);
            double x = xxa * c - zza * s, z = zza * c + xxa * s;
            double input = Mth.square(x) + Mth.square(z), moved = Mth.square(movement.x) + Mth.square(movement.z);
            if (input < 1.0E-5F || moved < 1.0E-5F) return false;
            return Math.acos((x * movement.x + z * movement.z) / Math.sqrt(input * moved)) < 0.13962634F;
        }
    }

    static MinecraftServer findServer() throws Exception {
        for (Thread thread : Thread.getAllStackTraces().keySet()) {
            if (!thread.getName().equals("Server thread")) continue;
            Field holder = Thread.class.getDeclaredField("holder");
            holder.setAccessible(true);
            Object h = holder.get(thread);
            Field task = h.getClass().getDeclaredField("task");
            task.setAccessible(true);
            Object runnable = task.get(h);
            for (Field f : runnable.getClass().getDeclaredFields()) {
                f.setAccessible(true);
                if (f.get(runnable) instanceof AtomicReference<?> ref && ref.get() instanceof MinecraftServer s) return s;
            }
        }
        return null;
    }

    static BlockState parseState(String text) {
        int open = text.indexOf('[');
        String name = open < 0 ? text : text.substring(0, open);
        Block block = BuiltInRegistries.BLOCK.getOptional(Identifier.parse(name)).orElseThrow(() -> new IllegalArgumentException(name));
        BlockState state = block.defaultBlockState();
        if (open >= 0) {
            for (String pair : text.substring(open + 1, text.length() - 1).split(",")) {
                String[] kv = pair.split("=");
                state = with(state, block.getStateDefinition().getProperty(kv[0]), kv[1]);
            }
        }
        return state;
    }
    static <T extends Comparable<T>> BlockState with(BlockState state, Property<T> property, String value) {
        if (property == null) throw new IllegalArgumentException("property " + value);
        return state.setValue(property, property.getValue(value).orElseThrow(() -> new IllegalArgumentException(value)));
    }

    // Exact decimal text (shortest round-trip form); parse with a correctly rounding reader.
    static <T extends Comparable<T>> String valueName(BlockState state, Property<T> property) {
        return property.getName(state.getValue(property));
    }

    static JsonArray vec(double... xs) { JsonArray a = new JsonArray(); for (double x : xs) a.add(Double.toString(x)); return a; }

    static final String VERSION_NAME = "1.21.11";
    static Iterable<Block> blocks() { return BuiltInRegistries.BLOCK; }
    static String blockName(Block block) { return BuiltInRegistries.BLOCK.getKey(block).toString(); }
    static boolean hasTag(Block block, net.minecraft.tags.TagKey<Block> tag) { return block.defaultBlockState().is(tag); }

    // Block audit: per-state collision boxes and flags, per-block movement hooks.
    // `spec` lists hook=baseClass:method:parameterTypes in official names (from run.py).
    static JsonObject exportBlocks(String spec) throws Exception {
        Map<String, Method> hooks = new LinkedHashMap<>();
        for (String entry : spec.split(";")) {
            String[] kv = entry.split("="), p = kv[1].split(":");
            Method found = null;
            for (Method m : Class.forName(p[0]).getDeclaredMethods()) {
                String types = String.join(",", Arrays.stream(m.getParameterTypes()).map(Class::getName).toList());
                if (m.getName().equals(p[1]) && types.equals(p.length > 2 ? p[2] : "")) {
                    if (found != null) throw new IllegalStateException("ambiguous " + entry);
                    found = m;
                }
            }
            if (found == null) throw new IllegalStateException("missing " + entry);
            hooks.put(kv[0], found);
        }
        JsonArray shapes = new JsonArray();
        Map<String, Integer> shapeIndex = new HashMap<>();
        JsonArray stateShape = new JsonArray(), fluid = new JsonArray(), fluids = new JsonArray(), blocksMotion = new JsonArray(), sturdy = new JsonArray(), suffocating = new JsonArray(), positional = new JsonArray();
        BlockPos other = new BlockPos(7, 64, -13);
        int count = 0;
        for (int id = 0; ; id++) {
            BlockState state = Block.BLOCK_STATE_REGISTRY.byId(id);
            if (state == null) break;
            count++;
            // The context-taking form is what entity collision calls (never the cached shape).
            JsonArray boxes = boxes(state.getCollisionShape(EmptyBlockGetter.INSTANCE, BlockPos.ZERO, CollisionContext.empty()));
            if (!boxes.equals(boxes(state.getCollisionShape(EmptyBlockGetter.INSTANCE, other, CollisionContext.empty())))) positional.add(id);
            String key = boxes.toString();
            Integer index = shapeIndex.get(key);
            if (index == null) { index = shapes.size(); shapeIndex.put(key, index); shapes.add(boxes); }
            stateShape.add(index);
            FluidState fs = state.getFluidState();
            if (!fs.isEmpty()) {
                fluid.add(id);
                // [state, 1 water / 2 lava, amount, falling]
                boolean falling = fs.getProperties().contains(FlowingFluid.FALLING) && fs.getValue(FlowingFluid.FALLING);
                fluids.add(ints(id, fs.is(FluidTags.WATER) ? 1 : 2, fs.getAmount(), falling ? 1 : 0));
            }
            if (state.blocksMotion()) blocksMotion.add(id);
            int faces = 0;
            Direction[] horizontal = {Direction.NORTH, Direction.EAST, Direction.SOUTH, Direction.WEST};
            for (int d = 0; d < 4; d++) if (state.isFaceSturdy(EmptyBlockGetter.INSTANCE, BlockPos.ZERO, horizontal[d])) faces |= 1 << d;
            // Flow ignores the faces of ice (FlowingFluid.isSolidFace).
            if (state.getBlock() instanceof net.minecraft.world.level.block.IceBlock) faces |= 16;
            if (faces != 0) sturdy.add(ints(id, faces));
            if (state.isSuffocating(EmptyBlockGetter.INSTANCE, BlockPos.ZERO)) suffocating.add(id);
        }
        JsonArray blocks = new JsonArray();
        for (Block block : blocks()) {
            JsonObject b = new JsonObject();
            b.addProperty("name", blockName(block));
            b.addProperty("class", block.getClass().getName());
            b.addProperty("first_state", Block.getId(block.getStateDefinition().getPossibleStates().get(0)));
            b.addProperty("states", block.getStateDefinition().getPossibleStates().size());
            b.addProperty("friction", Float.toString(block.getFriction()));
            b.addProperty("speed_factor", Float.toString(block.getSpeedFactor()));
            b.addProperty("jump_factor", Float.toString(block.getJumpFactor()));
            JsonArray tags = new JsonArray();
            if (hasTag(block, BlockTags.CLIMBABLE)) tags.add("climbable");
            if (hasTag(block, BlockTags.FENCES)) tags.add("fences");
            if (hasTag(block, BlockTags.WALLS)) tags.add("walls");
            if (block instanceof FenceGateBlock) tags.add("fence_gate");
            if (block instanceof TrapDoorBlock) tags.add("trapdoor");
            b.add("tags", tags);
            JsonObject implemented = new JsonObject();
            for (Map.Entry<String, Method> hook : hooks.entrySet()) {
                Class<?> base = hook.getValue().getDeclaringClass();
                for (Class<?> c = block.getClass(); c != base && c != null; c = c.getSuperclass()) {
                    try {
                        c.getDeclaredMethod(hook.getValue().getName(), hook.getValue().getParameterTypes());
                        implemented.addProperty(hook.getKey(), c.getName());
                        break;
                    } catch (NoSuchMethodException ignored) {
                    }
                }
            }
            b.add("hooks", implemented);
            blocks.add(b);
        }
        JsonObject out = new JsonObject();
        out.addProperty("state_count", count);
        out.add("shapes", shapes);
        out.add("state_shapes", stateShape);
        out.add("fluid_states", fluid);
        out.add("fluids", fluids);
        out.add("blocks_motion_states", blocksMotion);
        out.add("sturdy_faces", sturdy);
        out.add("suffocating_states", suffocating);
        out.add("positional_shape_states", positional);
        JsonObject attributeIds = new JsonObject(), effectIds = new JsonObject();
        for (var a : BuiltInRegistries.ATTRIBUTE) attributeIds.addProperty(BuiltInRegistries.ATTRIBUTE.getKey(a).toString(), BuiltInRegistries.ATTRIBUTE.getId(a));
        for (var e : BuiltInRegistries.MOB_EFFECT) effectIds.addProperty(BuiltInRegistries.MOB_EFFECT.getKey(e).toString(), BuiltInRegistries.MOB_EFFECT.getId(e));
        out.add("attribute_ids", attributeIds);
        out.add("effect_ids", effectIds);
        out.add("blocks", blocks);
        return out;
    }
    static JsonArray ints(int... xs) { JsonArray a = new JsonArray(); for (int x : xs) a.add(x); return a; }
    static JsonArray boxes(VoxelShape shape) {
        JsonArray list = new JsonArray();
        for (AABB box : shape.toAabbs()) list.add(vec(box.minX, box.minY, box.minZ, box.maxX, box.maxY, box.maxZ));
        return list;
    }

    static final int CLEAR = 12;

    static JsonObject scenario(ServerLevel level, BlockPos origin, JsonObject scenario) {
        for (int cx = (origin.getX() - CLEAR) >> 4; cx <= (origin.getX() + CLEAR) >> 4; cx++)
            for (int cz = (origin.getZ() - CLEAR) >> 4; cz <= (origin.getZ() + CLEAR) >> 4; cz++)
                level.getChunk(cx, cz);
        BlockState air = net.minecraft.world.level.block.Blocks.AIR.defaultBlockState();
        for (BlockPos p : BlockPos.betweenClosed(origin.offset(-CLEAR, -CLEAR, -CLEAR), origin.offset(CLEAR, CLEAR, CLEAR)))
            level.setBlock(p, air, 18);
        JsonObject resolved = new JsonObject();
        for (JsonElement e : scenario.getAsJsonArray("blocks")) {
            JsonArray b = e.getAsJsonArray();
            BlockState state = parseState(b.get(6).getAsString());
            JsonObject properties = new JsonObject();
            for (Property<?> property : state.getProperties()) properties.addProperty(property.getName(), valueName(state, property));
            resolved.add(b.get(6).getAsString(), properties);
            for (int x = b.get(0).getAsInt(); x <= b.get(3).getAsInt(); x++)
                for (int y = b.get(1).getAsInt(); y <= b.get(4).getAsInt(); y++)
                    for (int z = b.get(2).getAsInt(); z <= b.get(5).getAsInt(); z++)
                        level.setBlock(origin.offset(x, y, z), state, 18);
        }
        OraclePlayer player = new OraclePlayer(level);
        JsonArray start = scenario.getAsJsonArray("start");
        player.setPos(origin.getX() + start.get(0).getAsDouble(), origin.getY() + start.get(1).getAsDouble(), origin.getZ() + start.get(2).getAsDouble());
        player.setOnGround(scenario.has("on_ground") ? scenario.get("on_ground").getAsBoolean() : true);
        if (scenario.has("boat")) return boatScenario(level, origin, scenario, resolved, player);
        if (scenario.has("attributes")) {
            for (Map.Entry<String, JsonElement> a : scenario.getAsJsonObject("attributes").entrySet()) {
                var holder = BuiltInRegistries.ATTRIBUTE.get(Identifier.parse(a.getKey())).orElseThrow();
                player.getAttribute(holder).setBaseValue(a.getValue().getAsDouble());
            }
        }
        if (scenario.has("attribute_modifiers")) {
            for (JsonElement e : scenario.getAsJsonArray("attribute_modifiers")) {
                JsonObject m=e.getAsJsonObject();
                var holder=BuiltInRegistries.ATTRIBUTE.get(Identifier.parse(m.get("attribute").getAsString())).orElseThrow();
                player.getAttribute(holder).addTransientModifier(new net.minecraft.world.entity.ai.attributes.AttributeModifier(
                    Identifier.parse(m.get("id").getAsString()),m.get("amount").getAsDouble(),
                    net.minecraft.world.entity.ai.attributes.AttributeModifier.Operation.valueOf(m.get("operation").getAsString())));
            }
        }
        if (scenario.has("effects")) {
            for (Map.Entry<String, JsonElement> a : scenario.getAsJsonObject("effects").entrySet()) {
                var holder = BuiltInRegistries.MOB_EFFECT.get(Identifier.parse(a.getKey())).orElseThrow();
                player.addEffect(new MobEffectInstance(holder, 100000, a.getValue().getAsInt()));
            }
        }
        JsonObject initial = new JsonObject();
        var speed = player.getAttribute(net.minecraft.world.entity.ai.attributes.Attributes.MOVEMENT_SPEED);
        initial.addProperty("movement_speed_base", Double.toString(speed.getBaseValue()));
        JsonArray modifiers = new JsonArray();
        for (var m : speed.getModifiers()) {
            JsonObject o = new JsonObject();
            o.addProperty("id", m.id().toString());
            o.addProperty("operation", m.operation().name());
            o.addProperty("amount", Double.toString(m.amount()));
            modifiers.add(o);
        }
        initial.add("movement_speed_modifiers", modifiers);
        for (var name : List.of("jump_strength", "step_height", "gravity", "sneaking_speed", "movement_efficiency", "water_movement_efficiency")) {
            var holder = BuiltInRegistries.ATTRIBUTE.get(Identifier.withDefaultNamespace(name)).orElseThrow();
            initial.addProperty(name, Double.toString(player.getAttributeValue(holder)));
        }
        initial.addProperty("food", player.getFoodData().getFoodLevel());
        JsonArray frames = new JsonArray();
        for (JsonElement e : scenario.getAsJsonArray("ticks")) {
            JsonObject t = e.getAsJsonObject();
            int forward = t.has("forward") ? t.get("forward").getAsInt() : 0;
            int strafe = t.has("strafe") ? t.get("strafe").getAsInt() : 0;
            player.next = new Input(forward > 0, forward < 0, strafe > 0, strafe < 0,
                t.has("jump") && t.get("jump").getAsBoolean(), t.has("sneak") && t.get("sneak").getAsBoolean(),
                t.has("sprint") && t.get("sprint").getAsBoolean());
            // Item use as the client starts it (Minecraft.handleKeybinds runs before the player tick).
            String using = t.has("using") ? t.get("using").getAsString() : null;
            if (using != null && !player.isUsingItem()) {
                player.setItemInHand(net.minecraft.world.InteractionHand.MAIN_HAND,
                    new net.minecraft.world.item.ItemStack(BuiltInRegistries.ITEM.getValue(Identifier.parse(using))));
                player.startUsingItem(net.minecraft.world.InteractionHand.MAIN_HAND);
            } else if (using == null && player.isUsingItem()) {
                player.stopUsingItem();
            }
            player.setYRot(t.get("yaw").getAsFloat());
            player.setXRot(t.has("pitch") ? t.get("pitch").getAsFloat() : 0.0F);
            player.tick();
            Vec3 v = player.getDeltaMovement();
            JsonObject f = new JsonObject();
            f.add("position", vec(player.getX() - origin.getX(), player.getY() - origin.getY(), player.getZ() - origin.getZ()));
            f.add("velocity", vec(v.x, v.y, v.z));
            f.addProperty("on_ground", player.onGround());
            f.addProperty("horizontal_collision", player.horizontalCollision);
            f.addProperty("sprinting", player.isSprinting());
            f.addProperty("crouching", player.crouching);
            f.addProperty("pose", player.getPose().name());
            f.addProperty("in_water", player.isInWater());
            f.addProperty("swimming", player.isSwimming());
            f.addProperty("using", player.isUsingItem());
            f.addProperty("speed", Float.toString(player.getSpeed()));
            f.addProperty("fall_distance", Double.toString(player.fallDistance));
            frames.add(f);
        }
        JsonObject out = new JsonObject();
        out.addProperty("name", scenario.get("name").getAsString());
        out.add("initial", initial);
        out.add("states", resolved);
        out.add("frames", frames);
        return out;
    }

    // The unchanged native boat methods run inside the original ServerLevel.
    // Only status/input scheduling is supplied; no vehicle method is replaced.
    static JsonObject boatScenario(ServerLevel level, BlockPos origin, JsonObject scenario,
                                   JsonObject resolved, OraclePlayer player) {
        try {
            var boat = new net.minecraft.world.entity.vehicle.boat.Boat(net.minecraft.world.entity.EntityType.OAK_BOAT, level, () -> net.minecraft.world.item.Items.OAK_BOAT);
            boat.setPos(player.getX(), player.getY(), player.getZ());
            boat.setYRot(scenario.has("yaw") ? scenario.get("yaw").getAsFloat() : 0.0F);
            // Explicit stationary native fixtures: their world collision
            // queries run unchanged; no entity predicate/geometry is overridden.
            List<net.minecraft.world.entity.Entity> obstacles = new java.util.ArrayList<>();
            JsonArray collisionBoxes = new JsonArray();
            if (scenario.has("collision_bodies")) for (JsonElement element : scenario.getAsJsonArray("collision_bodies")) {
                JsonObject inputBody = element.getAsJsonObject();
                net.minecraft.world.entity.Entity other = inputBody.get("type").getAsString().equals("minecart") ? new net.minecraft.world.entity.vehicle.minecart.Minecart(net.minecraft.world.entity.EntityType.MINECART, level) : new net.minecraft.world.entity.vehicle.boat.Boat(net.minecraft.world.entity.EntityType.OAK_BOAT, level, () -> net.minecraft.world.item.Items.OAK_BOAT);
                JsonArray at = inputBody.getAsJsonArray("position");
                other.setPos(origin.getX()+at.get(0).getAsDouble(),origin.getY()+at.get(1).getAsDouble(),origin.getZ()+at.get(2).getAsDouble());
                if (!level.addFreshEntity(other)) throw new IllegalStateException("native collision fixture not registered");
                obstacles.add(other);
                var box = other.getBoundingBox();
                collisionBoxes.add(vec(box.minX-origin.getX(),box.minY-origin.getY(),box.minZ-origin.getZ(),box.maxX-origin.getX(),box.maxY-origin.getY(),box.maxZ-origin.getZ()));
            }
            for (var other : obstacles) if (!level.getEntities(boat, other.getBoundingBox().inflate(0.1)).contains(other))
                throw new IllegalStateException("declared native collision fixture not visible to original world query");
            player.startRiding(boat, true, false);
            if (scenario.has("velocity")) {
                JsonArray v = scenario.getAsJsonArray("velocity");
                boat.setDeltaMovement(new Vec3(v.get(0).getAsDouble(), v.get(1).getAsDouble(), v.get(2).getAsDouble()));
            }
            Class<?> base = net.minecraft.world.entity.vehicle.boat.AbstractBoat.class;
            // Names from the pinned official mappings: reflection strings are
            // not changed when SpecialSource remaps the harness bytecode.
            Field status = base.getDeclaredField("aZ"), old = base.getDeclaredField("ba"), angular = base.getDeclaredField("aR");
            for (Field f : List.of(status, old, angular)) f.setAccessible(true);
            Method get = base.getDeclaredMethod("H"), floating = base.getDeclaredMethod("K"), control = base.getDeclaredMethod("L");
            for (Method m : List.of(get, floating, control)) m.setAccessible(true);
            JsonArray frames = new JsonArray();
            for (JsonElement e : scenario.getAsJsonArray("ticks")) {
                JsonObject t = e.getAsJsonObject();
                if (t.has("received_boat_velocity")) {
                    JsonArray v = t.getAsJsonArray("received_boat_velocity");
                    boat.setDeltaMovement(new Vec3(v.get(0).getAsDouble(), v.get(1).getAsDouble(), v.get(2).getAsDouble()));
                }
                int forward = t.has("forward") ? t.get("forward").getAsInt() : 0;
                int strafe = t.has("strafe") ? t.get("strafe").getAsInt() : 0;
                old.set(boat, status.get(boat));
                status.set(boat, get.invoke(boat));
                boat.baseTick();
                boat.setInput(strafe > 0, strafe < 0, forward > 0, forward < 0);
                floating.invoke(boat);
                control.invoke(boat);
                boat.move(net.minecraft.world.entity.MoverType.SELF, boat.getDeltaMovement());
                if (scenario.has("boat_bubbles") || scenario.has("boat_hooks")) {
                    // AbstractBoat.tick invokes the unchanged Entity collector
                    // twice. Surface launch/ejection is server-owned and is
                    // qualified by real packets, not called as client physics.
                    Method effects = net.minecraft.world.entity.Entity.class.getDeclaredMethod("aW");
                    effects.setAccessible(true);
                    effects.invoke(boat);
                    effects.invoke(boat);
                }
                Vec3 v = boat.getDeltaMovement();
                JsonObject f = new JsonObject();
                f.add("position", vec(boat.getX()-origin.getX(), boat.getY()-origin.getY(), boat.getZ()-origin.getZ()));
                f.add("velocity", vec(v.x, v.y, v.z));
                f.add("rotation", vec(boat.getYRot(), boat.getXRot()));
                f.addProperty("angular_velocity", Float.toString(angular.getFloat(boat)));
                f.addProperty("on_ground", boat.onGround());
                String waterStatus = status.get(boat).toString();
                f.addProperty("water_status", waterStatus);
                f.addProperty("in_water", waterStatus.equals("IN_WATER") || waterStatus.equals("UNDER_WATER") || waterStatus.equals("UNDER_FLOWING_WATER"));
                JsonArray paddles = new JsonArray();
                paddles.add(boat.getPaddleState(0)); paddles.add(boat.getPaddleState(1));
                f.add("paddles", paddles);
                frames.add(f);
            }
            player.stopRiding();
            for (var other : obstacles) other.discard();
            JsonObject out = new JsonObject();
            out.addProperty("name", scenario.get("name").getAsString());
            if (scenario.has("collision_bodies")) out.add("collision_boxes", collisionBoxes);
            out.add("states", resolved); out.add("frames", frames);
            return out;
        } catch (Exception e) { throw new RuntimeException(e); }
    }

    public static void main(String[] args) {
        try {
            run(args);
        } catch (Throwable e) {
            e.printStackTrace();
            System.exit(1);
        }
    }

    static void run(String[] args) throws Exception {
        JsonArray scenarios = args[0].equals("--blocks") ? null : JsonParser.parseString(Files.readString(Path.of(args[0]))).getAsJsonArray();
        net.minecraft.server.Main.main(new String[]{"--nogui"});
        MinecraftServer server = null;
        for (int i = 0; i < 600 && (server == null || server.getTickCount() < 20); i++) {
            Thread.sleep(500);
            if (server == null) server = findServer();
        }
        if (server == null || server.getTickCount() < 20) throw new IllegalStateException("server did not start");
        if (args[0].equals("--blocks")) {
            JsonObject[] audit = new JsonObject[1];
            server.executeBlocking(() -> {
                try {
                    audit[0] = exportBlocks(System.getProperty("voxrig.hooks"));
                } catch (Throwable e) {
                    e.printStackTrace();
                    System.exit(1);
                }
            });
            audit[0].addProperty("version", VERSION_NAME);
            Files.writeString(Path.of(args[1]), new GsonBuilder().create().toJson(audit[0]) + "\n");
            System.out.println("ORACLE DONE blocks");
            server.halt(false);
            System.exit(0);
        }
        JsonArray results = new JsonArray();
        MinecraftServer s = server;
        boolean collisionScenes = false;
        for (JsonElement e : scenarios) collisionScenes |= e.getAsJsonObject().has("collision_bodies");
        if (collisionScenes) {
            // Wait on the original server's actual entity-ticking chunk state,
            // outside its thread. Hidden entity sections are not a collision oracle.
            s.executeBlocking(() -> {
                for (int x=63;x<=65;x++) for (int z=63;z<=65;z++) {
                    s.overworld().setChunkForced(x,z,true);
                    s.overworld().getChunk(x,z);
                }
            });
            java.util.concurrent.atomic.AtomicBoolean ready = new java.util.concurrent.atomic.AtomicBoolean(false);
            for (int attempt=0;attempt<600 && !ready.get();attempt++) {
                s.executeBlocking(() -> ready.set(s.overworld().isPositionEntityTicking(new BlockPos(1024,100,1024))));
                if (!ready.get()) Thread.sleep(50);
            }
            if (!ready.get()) throw new IllegalStateException("native collision fixture chunk is not entity ticking");
        }
        server.executeBlocking(() -> {
            ServerLevel level = s.overworld();
            BlockPos origin = new BlockPos(1024, 100, 1024);
            try {
                for (JsonElement e : scenarios) results.add(scenario(level, origin, e.getAsJsonObject()));
            } catch (Throwable e) {
                e.printStackTrace();
                System.exit(1);
            }
        });
        JsonObject out = new JsonObject();
        out.addProperty("version", net.minecraft.SharedConstants.getCurrentVersion().name());
        out.add("results", results);
        Files.writeString(Path.of(args[1]), new GsonBuilder().create().toJson(out) + "\n");
        System.out.println("ORACLE DONE " + results.size());
        server.halt(false);
        System.exit(0);
    }
}
