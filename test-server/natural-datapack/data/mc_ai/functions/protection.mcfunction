function mc_ai:register_bots
effect give @a[tag=mc_ai_bot] minecraft:resistance 5 4 true
effect give @a[tag=mc_ai_bot] minecraft:water_breathing 5 0 true
schedule function mc_ai:protection 20t replace
