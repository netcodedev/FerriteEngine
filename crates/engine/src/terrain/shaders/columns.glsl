// Pictures of chunk columns around the camera (see `terrain::columns`):
// a texel per `columnSize` metre square, 1 where the column is in the set,
// the first texel the column `origin`.

uniform float columnSize;

bool columnIn(sampler2D map, ivec2 origin, vec2 xz) {
    ivec2 texel = ivec2(floor(xz / columnSize)) - origin;
    ivec2 side = textureSize(map, 0);
    return all(greaterThanEqual(texel, ivec2(0))) && all(lessThan(texel, side))
        && texelFetch(map, texel, 0).r > 0.5;
}

// The columns whose chunks are loaded: the horizon and the far water leave
// them to the chunks.
uniform sampler2D loadedColumns;
uniform ivec2 loadedOrigin;

bool columnLoaded(vec2 xz) {
    return columnIn(loadedColumns, loadedOrigin, xz);
}
