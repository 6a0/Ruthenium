<p align="center">
  <img src="./src/main/resources/assets/ruthenium/rlogo.png" width="125" alt="Ruthenium logo"/>
</p>

<h1 align="center">Ruthenium</h1>

<p align="center">
  An optimisation mod designed to consolidate previous efforts into a more compact and performant mod, plus some extras.
</p>

<p align="center">
  <b>26.3 only, for now :)</b>
</p>

> [!NOTE]
> AI was used heavily for this project.

## Recommended JVM arguments

Minecraft 26.3 runs on Java 25, which has two options that save memory. A mod can't set them, so add them to your
launcher's JVM arguments:

```
-XX:+UseCompactObjectHeaders -XX:+UseStringDeduplication
```

- `-XX:+UseCompactObjectHeaders` shrinks every Java object's header from 12 to 8 bytes. In world at render distance 12
  this measured about 25 MB less live heap.
- `-XX:+UseStringDeduplication` lets the G1 garbage collector (the default) share the contents of identical strings.
  This measured about 12 MB less in world.

## Credits

- [xCollateral](https://github.com/xCollateral/) – for the base of the mod ([VulkanMod](https://github.com/xCollateral/VulkanMod)) and extensive existing work on Vulkan
- [CaffeineMC](https://github.com/CaffeineMC) and contributors – for [Lithium](https://github.com/CaffeineMC/lithium) and [Sodium](https://github.com/CaffeineMC/sodium)'s extensive existing work on client ticking/rendering
