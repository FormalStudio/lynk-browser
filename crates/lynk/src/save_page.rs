//! Save Page As, Cmd+S: Chrome's way, the page's own address downloaded again with the page's
//! profile and cookies, always through the save panel, named after its title. HTML only, no
//! folder of resources. It answers only the downloads it started; any other download is not its
//! business.

use std::collections::HashMap;

use lynk_core::downloads::saved_page_name;
use nd::components::SaveDialog;
use nd::prelude::*;

use crate::page::PageEvent;
use crate::tab::TabId;
use crate::window::BrowserWindow;

/// The pages being saved, by tab: the address asked for and the title the file is named after.
#[derive(Default)]
struct Saving(HashMap<TabId, (String, String)>);

/// The Save Page As command.
pub fn save(win: &mut BrowserWindow, id: TabId, _: &mut Window, cx: &mut Context<BrowserWindow>) {
    let Some(tab) = win.tab(id).filter(|t| t.is_web_page()) else { return };
    let Some(view) = tab.view() else { return };
    if !cx.has_global::<Saving>() {
        cx.set_global(Saving::default());
    }
    cx.global_mut::<Saving>().0.insert(id, (tab.url.clone(), tab.title.clone()));
    view.start_download(tab.url.clone());
}

/// A page observer: the download a save started goes where the save panel says, or nowhere.
pub fn observe(win: &mut BrowserWindow, id: TabId, event: &PageEvent, _: &mut Window, cx: &mut Context<BrowserWindow>) {
    let PageEvent::DownloadRequested(e) = event else { return };
    let Some(saving) = cx.try_global_mut::<Saving>() else { return };
    if saving.0.get(&id).is_none_or(|(url, _)| *url != e.download.url) {
        return;
    }
    let Some((url, title)) = saving.0.remove(&id) else { return };
    let suggested = Some(e.download.suggested_filename.as_str()).filter(|s| !s.is_empty());
    let dialog = SaveDialog::new().file_name(saved_page_name(&title, &url, suggested));
    let download = e.download.clone();
    let picked = nd::system::window::save(cx, win.id, dialog);
    nd::system::spawn(picked, move |path, _| match path {
        Some(path) => download.save_to(path),
        None => download.cancel(),
    });
}

/// A tab listener: a closed tab saves nothing more.
pub fn tab_event(_: &mut BrowserWindow, id: TabId, event: crate::window::TabEvent, cx: &mut Context<BrowserWindow>) {
    if event == crate::window::TabEvent::Closed
        && let Some(s) = cx.try_global_mut::<Saving>()
    {
        s.0.remove(&id);
    }
}
