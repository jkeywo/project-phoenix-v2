// All use opaque sRGB 4096x2048 bases and 13 mips. Other material layers retain
// their existing formats. The worker templates describe this shared layout.
export const maps = [
  { entity: 'planet_gas_giant', stem: 'gas_giant/surface_colour', textures: 11 },
  { entity: 'moon_ice', stem: 'ice_moon/surface_colour', textures: 11 },
  { entity: 'planet_ecumenopolis', stem: 'ecumenopolis/city_albedo', textures: 11 },
];
