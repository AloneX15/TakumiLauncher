<script>
  import fox from '../assets/takumi-fox.png';

  /** Size in px. */
  export let size = 38;
</script>

<!-- The fox PNG is used as a mask so it takes the palette's accent gradient. -->
<span class="logo" style="--size: {size}px; --fox: url({fox})" role="img" aria-label="Takumi" />

<style>
  .logo {
    display: block;
    flex-shrink: 0;
    width: var(--size);
    height: var(--size);
    background: linear-gradient(170deg, var(--accent-hover), var(--accent) 55%, #8f0d27);
    -webkit-mask: var(--fox) center / contain no-repeat;
    mask: var(--fox) center / contain no-repeat;
  }
</style>
