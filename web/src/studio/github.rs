use super::state::Studio;
use dioxus::prelude::*;
#[derive(Clone, Copy)]
pub struct Session {
    repository: Signal<String>,
    token: Signal<String>,
}
pub fn use_session() -> Session {
    Session {
        repository: use_signal(String::new),
        token: use_signal(String::new),
    }
}
#[component]
pub fn GitReview(mut studio: Studio) -> Element {
    let mut session = use_context::<Session>();
    let mut busy = use_signal(|| false);
    let mut message = use_signal(String::new);
    let mut new_name = use_signal(String::new);
    rsx! {details{summary{"Share with GitHub"}
        p{"Use a dedicated YACE review repository. Pull and publish proposals, comments and approvals directly through GitHub. Audio and project files stay local; incoming reviews never merge notes automatically."}
        label{r#for:"review-github-repo","GitHub review repository"}
        input{id:"review-github-repo",class:"field-control",value:"{session.repository}",placeholder:"owner/review-repository",disabled:busy(),oninput:move|e|session.repository.set(e.value())}
        label{r#for:"review-github-token","Optional GitHub token"}
        input{id:"review-github-token",r#type:"password",autocomplete:"off",class:"field-control",value:"{session.token}",disabled:busy(),oninput:move|e|session.token.set(e.value())}
        p{class:"field-hint","Public repositories can be read without signing in. Publishing needs a fine-grained token for this review repository with Contents: read and write. Authorization stays in this tab's memory and is excluded from project saves, downloads and browser storage."}
        button{disabled:busy(),onclick:move|_|session.token.set(String::new()),"Clear GitHub authorization"}
        for (publish,label) in [(false,"Pull GitHub reviews"),(true,"Save / Pull / Publish reviews")] {
            button{class:"field-button",disabled:busy()||(publish&&(session.token)().is_empty()),onclick:move|_|{
                let repository=(session.repository)();let token=(session.token)();
                let store=studio.store.read().clone();let doc=studio.snapshot();
                busy.set(true);message.set("Synchronizing immutable review history…".into());
                dioxus::core::spawn_forever(async move{
                    let result=async{
                        let store=store.ok_or_else(||anyhow::anyhow!("Open a project"))?;
                        let doc=doc.ok_or_else(||anyhow::anyhow!("Open a project"))?;
                        let repository=crate::review_repository::Repository::parse(&repository)?;
                        let api=crate::github_api::Client::new(token)?;
                        let bundle=crate::github_reviews::synchronize(&api,&repository,&doc,publish).await?;
                        anyhow::ensure!(studio.store.read().as_ref().is_some_and(|s|s.root==store.root),"The active project changed. Reviews remain on GitHub; pull into the intended project again.");
                        let current=studio.snapshot().ok_or_else(||anyhow::anyhow!("Open a project"))?;
                        if crate::collaboration_exchange::validate_import(&current,&bundle)? {
                            store.apply(crate::editor::Request{expected_revision:current.revision,actor:crate::editor::Actor{kind:crate::editor::ActorKind::Human,name:(studio.reviewer)()},action:crate::editor::Action::ImportCollaboration{bundle}})?;
                            studio.reload(false);studio.dirty.set(true);
                        }
                        Ok::<_,anyhow::Error>(())
                    }.await;
                    busy.set(false);
                    message.set(match result{Ok(())=>if publish{"Review history published; inspect and merge approved proposals explicitly.".into()}else{"Review history pulled; inspect proposals before merging.".into()},Err(error)=>format!("{error:#}")});
                });
            },"{label}"}
        }
        details{summary{"Create a private review repository"}
            p{"Creates a new private repository in your GitHub account. Creating repositories also needs GitHub's repository creation permission. An existing source repository will never be converted."}
            label{r#for:"review-github-new","New repository name"}
            input{id:"review-github-new",class:"field-control",value:"{new_name}",disabled:busy(),oninput:move|e|new_name.set(e.value())}
            button{disabled:busy()||(session.token)().is_empty()||new_name().trim().is_empty(),onclick:move|_|{
                let name=new_name();let token=(session.token)();busy.set(true);
                dioxus::core::spawn_forever(async move{
                    let result=async{let api=crate::github_api::Client::new(token)?;crate::github_reviews::create(&api,&name).await}.await;
                    busy.set(false);match result{Ok(repo)=>{session.repository.set(format!("{}/{}",repo.owner,repo.name));message.set("Private review repository created. Save / Pull / Publish to share your review history.".into());},Err(error)=>message.set(format!("{error:#}"))}
                });
            },"Create private GitHub review repository"}
        }
        p{role:"status","{message}"}
    }}
}
