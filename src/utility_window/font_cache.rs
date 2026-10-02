//! Scoped glyph-cache ownership shared by Settings and alarm utility windows.
use bevy::{
    asset::AssetId,
    prelude::*,
    text::{FontAtlasKey, FontAtlasSet, TextLayoutInfo},
};
use std::collections::HashSet;

#[derive(Component)]
pub(crate) struct FontCacheOwner;

#[derive(Resource, Default)]
pub(crate) struct UtilityFontCache {
    baseline: HashSet<FontAtlasKey>,
    active: bool,
}

impl UtilityFontCache {
    pub(crate) fn begin(&mut self, fonts: Option<&FontAtlasSet>) {
        // Overlapping windows share one lease until the last owned UI disappears.
        if self.active {
            return;
        }
        self.baseline = fonts.map_or_else(HashSet::new, |fonts| fonts.keys().copied().collect());
        self.active = true;
    }

    #[cfg(test)]
    pub(crate) fn is_active(&self) -> bool {
        self.active
    }
}

pub(crate) fn release(
    owners: Query<(), With<FontCacheOwner>>,
    layouts: Query<&TextLayoutInfo>,
    image_nodes: Query<&ImageNode>,
    mut fonts: Option<ResMut<FontAtlasSet>>,
    mut images: Option<ResMut<Assets<Image>>>,
    mut cache: ResMut<UtilityFontCache>,
) {
    // Windows may vanish in Last, before their camera/UI cleanup in the next Update.
    if !cache.active || !owners.is_empty() {
        return;
    }
    cache.active = false;
    if let (Some(fonts), Some(images)) = (fonts.as_mut(), images.as_mut()) {
        let used: HashSet<AssetId<Image>> = layouts
            .iter()
            .flat_map(|layout| layout.glyphs.iter().map(|glyph| glyph.atlas_info.texture))
            .chain(image_nodes.iter().map(|node| node.image.id()))
            .collect();
        fonts.retain(|key, atlases| {
            if cache.baseline.contains(key)
                || atlases
                    .iter()
                    .any(|atlas| used.contains(&atlas.texture.id()))
            {
                return true;
            }
            for atlas in atlases {
                images.remove(atlas.texture.id());
            }
            false
        });
    }
    cache.baseline = HashSet::new();
}
