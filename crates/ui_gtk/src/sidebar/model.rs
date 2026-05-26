// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use std::{collections::BTreeMap, rc::Rc};

use gtk::prelude::*;
use gtk::{gio, glib};
use sustain_app_runtime::{
    ApplicationRuntime, Playlist, PlaylistFolder, PlaylistFolderId, PlaylistItem, SmartPlaylist,
};

#[derive(Clone, Debug)]
pub(super) struct SidebarItem {
    pub(super) name: String,
    pub(super) item: PlaylistItem,
}

impl SidebarItem {
    pub(super) fn icon_name(&self) -> &'static str {
        match self.item {
            PlaylistItem::Folder(_) => "folder-symbolic",
            PlaylistItem::Playlist(_) => "view-list-symbolic",
            PlaylistItem::SmartPlaylist(_) => "emblem-system-symbolic",
        }
    }
}

#[derive(Default)]
struct SidebarSnapshot {
    items_by_parent: BTreeMap<Option<PlaylistFolderId>, Vec<SidebarItem>>,
}

impl SidebarSnapshot {
    fn from_runtime(runtime: &ApplicationRuntime) -> Self {
        Self::build(
            runtime.playlist_folders(),
            runtime.playlists(),
            runtime.smart_playlists(),
        )
    }

    fn build(
        folders: &[PlaylistFolder],
        playlists: &[Playlist],
        smart_playlists: &[SmartPlaylist],
    ) -> Self {
        let mut items_by_parent: BTreeMap<Option<PlaylistFolderId>, Vec<(u32, SidebarItem)>> =
            BTreeMap::new();

        for folder in folders {
            items_by_parent
                .entry(folder.parent_folder_id)
                .or_default()
                .push((
                    folder.position,
                    SidebarItem {
                        name: folder.name.clone(),
                        item: PlaylistItem::Folder(folder.id),
                    },
                ));
        }
        for playlist in playlists {
            items_by_parent
                .entry(playlist.parent_folder_id)
                .or_default()
                .push((
                    playlist.position,
                    SidebarItem {
                        name: playlist.name.clone(),
                        item: PlaylistItem::Playlist(playlist.id),
                    },
                ));
        }
        for smart in smart_playlists {
            items_by_parent
                .entry(smart.parent_folder_id)
                .or_default()
                .push((
                    smart.position,
                    SidebarItem {
                        name: smart.name.clone(),
                        item: PlaylistItem::SmartPlaylist(smart.id),
                    },
                ));
        }

        let items_by_parent = items_by_parent
            .into_iter()
            .map(|(parent, mut bucket)| {
                bucket.sort_by_key(|(position, _)| *position);
                (parent, bucket.into_iter().map(|(_, item)| item).collect())
            })
            .collect();

        Self { items_by_parent }
    }

    fn items_under(&self, parent: Option<PlaylistFolderId>) -> &[SidebarItem] {
        self.items_by_parent
            .get(&parent)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }
}

pub(super) fn selected_item(selection: &gtk::SingleSelection) -> Option<PlaylistItem> {
    let object = selection.selected_item()?;
    let tree_row = object.downcast_ref::<gtk::TreeListRow>()?;
    let inner = tree_row.item()?;
    let boxed = inner.downcast_ref::<glib::BoxedAnyObject>()?;
    let sidebar_item = boxed.try_borrow::<SidebarItem>().ok()?;
    Some(sidebar_item.item)
}

pub(super) fn select_item(selection: &gtk::SingleSelection, target: PlaylistItem) -> bool {
    let n = selection.n_items();
    for index in 0..n {
        let Some(object) = selection.item(index) else {
            continue;
        };
        let Some(tree_row) = object.downcast_ref::<gtk::TreeListRow>() else {
            continue;
        };
        let Some(inner) = tree_row.item() else {
            continue;
        };
        let Some(boxed) = inner.downcast_ref::<glib::BoxedAnyObject>() else {
            continue;
        };
        let Ok(sidebar_item) = boxed.try_borrow::<SidebarItem>() else {
            continue;
        };
        if sidebar_item.item == target {
            selection.set_selected(index);
            return true;
        }
    }
    selection.set_selected(gtk::INVALID_LIST_POSITION);
    false
}

pub(super) fn build_tree_model(runtime: &ApplicationRuntime) -> gtk::TreeListModel {
    let snapshot = Rc::new(SidebarSnapshot::from_runtime(runtime));
    let root_store = list_store_for(snapshot.items_under(None));

    let snapshot_for_children = snapshot.clone();
    gtk::TreeListModel::new(root_store, false, true, move |object| {
        children_for(object, &snapshot_for_children)
    })
}

fn children_for(object: &glib::Object, snapshot: &SidebarSnapshot) -> Option<gio::ListModel> {
    let boxed = object.downcast_ref::<glib::BoxedAnyObject>()?;
    let sidebar_item = boxed.try_borrow::<SidebarItem>().ok()?;
    let PlaylistItem::Folder(folder_id) = sidebar_item.item else {
        return None;
    };
    Some(list_store_for(snapshot.items_under(Some(folder_id))).upcast())
}

fn list_store_for(items: &[SidebarItem]) -> gio::ListStore {
    let store = gio::ListStore::new::<glib::BoxedAnyObject>();
    for item in items {
        store.append(&glib::BoxedAnyObject::new(item.clone()));
    }
    store
}

#[cfg(test)]
mod tests {
    use sustain_app_runtime::{
        PlaylistId, SmartPlaylistId, SmartPlaylistMatchKind, SmartPlaylistRule,
        SmartPlaylistRuleSet, SmartPlaylistTextField, SmartPlaylistTextOperator,
    };

    use super::*;

    fn folder(
        id: i64,
        name: &str,
        parent: Option<PlaylistFolderId>,
        position: u32,
    ) -> PlaylistFolder {
        PlaylistFolder {
            id: PlaylistFolderId::new(id).expect("positive folder id"),
            name: name.to_owned(),
            parent_folder_id: parent,
            position,
        }
    }

    fn playlist(id: i64, name: &str, parent: Option<PlaylistFolderId>, position: u32) -> Playlist {
        Playlist {
            id: PlaylistId::new(id).expect("positive playlist id"),
            name: name.to_owned(),
            parent_folder_id: parent,
            position,
            entries: Vec::new(),
        }
    }

    fn smart_playlist(
        id: i64,
        name: &str,
        parent: Option<PlaylistFolderId>,
        position: u32,
    ) -> SmartPlaylist {
        SmartPlaylist {
            id: SmartPlaylistId::new(id).expect("positive smart playlist id"),
            name: name.to_owned(),
            parent_folder_id: parent,
            position,
            rules: SmartPlaylistRuleSet {
                match_kind: SmartPlaylistMatchKind::All,
                rules: vec![SmartPlaylistRule::Text {
                    field: SmartPlaylistTextField::Genre,
                    operator: SmartPlaylistTextOperator::Is,
                    value: "Trip-Hop".to_owned(),
                }],
                limit: None,
            },
        }
    }

    #[test]
    fn snapshot_groups_items_by_parent_and_orders_them_by_position() {
        let root_folder = folder(1, "Mixes", None, 1);
        let root_playlist = playlist(1, "Drive", None, 0);
        let root_smart = smart_playlist(1, "Recent", None, 2);
        let nested_playlist = playlist(2, "Inside", Some(root_folder.id), 0);

        let snapshot = SidebarSnapshot::build(
            std::slice::from_ref(&root_folder),
            &[root_playlist.clone(), nested_playlist.clone()],
            std::slice::from_ref(&root_smart),
        );

        let root_items: Vec<PlaylistItem> = snapshot
            .items_under(None)
            .iter()
            .map(|item| item.item)
            .collect();
        assert_eq!(
            root_items,
            vec![
                PlaylistItem::Playlist(root_playlist.id),
                PlaylistItem::Folder(root_folder.id),
                PlaylistItem::SmartPlaylist(root_smart.id),
            ]
        );

        let nested_items: Vec<PlaylistItem> = snapshot
            .items_under(Some(root_folder.id))
            .iter()
            .map(|item| item.item)
            .collect();
        assert_eq!(
            nested_items,
            vec![PlaylistItem::Playlist(nested_playlist.id)]
        );

        assert!(
            snapshot
                .items_under(Some(PlaylistFolderId::new(999).expect("positive id")))
                .is_empty()
        );
    }
}
