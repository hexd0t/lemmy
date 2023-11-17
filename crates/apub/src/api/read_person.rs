use crate::{fetcher::resolve_actor_identifier, objects::person::ApubPerson};
use activitypub_federation::config::Data;
use actix_web::web::{Json, Query};
use lemmy_api_common::{
  context::LemmyContext,
  person::{GetPersonDetails, GetPersonDetailsResponse},
  utils::check_private_instance_filtered,
};
use lemmy_db_schema::{
  source::{local_site::LocalSite, person::Person},
  utils::post_to_comment_sort_type,
};
use lemmy_db_views::{comment_view::CommentQuery, post_view::PostQuery, structs::LocalUserView};
use lemmy_db_views_actor::structs::{CommunityModeratorView, PersonView};
use lemmy_utils::error::{LemmyError, LemmyErrorExt2, LemmyErrorType};

#[tracing::instrument(skip(context))]
pub async fn read_person(
  data: Query<GetPersonDetails>,
  context: Data<LemmyContext>,
  local_user_view: Option<LocalUserView>,
) -> Result<Json<GetPersonDetailsResponse>, LemmyError> {
  // Check to make sure a person name or an id is given
  if data.username.is_none() && data.person_id.is_none() {
    Err(LemmyErrorType::NoIdGiven)?
  }

  let local_site = LocalSite::read(&mut context.pool()).await?;
  let community_id = data.community_id;

  // If we're semi-private (private with federation), filter the posts & comments to only specifically requested communities,
  // and don't let the remote party know if we know about a community locally
  let filter = check_private_instance_filtered(
    &local_user_view,
    &local_site,
    &mut context.pool(),
    &community_id,
  )
  .await
  .map_err(|e| {
    tracing::warn!(
      "Denying APub resolve_object for {:?} on {:?} / {:?}",
      community_id,
      data.person_id,
      data.username
    );
    e
  })?;

  let person_details_id = match data.person_id {
    Some(id) => id,
    None => {
      if let Some(username) = &data.username {
        resolve_actor_identifier::<ApubPerson, Person>(username, &context, &local_user_view, true)
          .await
          .with_lemmy_type(LemmyErrorType::CouldntFindPerson)?
          .id
      } else {
        Err(LemmyErrorType::CouldntFindPerson)?
      }
    }
  };

  // You don't need to return settings for the user, since this comes back with GetSite
  // `my_user`
  let person_view = PersonView::read(&mut context.pool(), person_details_id).await?;

  let sort = data.sort;
  let page = data.page;
  let limit = data.limit;
  let saved_only = data.saved_only.unwrap_or_default();
  // If its saved only, you don't care what creator it was
  // Or, if its not saved, then you only want it for that specific creator
  let creator_id = if !saved_only {
    Some(person_details_id)
  } else {
    None
  };

  if filter {
    tracing::warn!(
      "Filtering APub read_person for {:?} on {:?}",
      community_id,
      person_details_id
    );
    return Ok(Json(GetPersonDetailsResponse {
      person_view,
      moderates: Vec::with_capacity(0),
      comments: Vec::with_capacity(0),
      posts: Vec::with_capacity(0),
    }));
  };
  let posts = PostQuery {
    sort,
    saved_only,
    local_user: local_user_view.as_ref(),
    community_id,
    is_profile_view: true,
    page,
    limit,
    creator_id,
    ..Default::default()
  }
  .list(&mut context.pool())
  .await?;

  let comments = CommentQuery {
    local_user: local_user_view.as_ref(),
    sort: sort.map(post_to_comment_sort_type),
    saved_only,
    community_id,
    is_profile_view: true,
    page,
    limit,
    creator_id,
    ..Default::default()
  }
  .list(&mut context.pool())
  .await?;

  let moderates = if let Some(community_filter) = community_id {
    CommunityModeratorView::for_community_and_person(
      &mut context.pool(),
      community_filter,
      person_details_id,
    )
    .await?
  } else {
    CommunityModeratorView::for_person(&mut context.pool(), person_details_id).await?
  };

  // Return the jwt
  Ok(Json(GetPersonDetailsResponse {
    person_view,
    moderates,
    comments,
    posts,
  }))
}
