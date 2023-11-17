use crate::{
  api::listing_type_with_default, fetcher::resolve_actor_identifier,
  objects::community::ApubCommunity,
};
use activitypub_federation::config::Data;
use actix_web::web::{Json, Query};
use lemmy_api_common::{
  context::LemmyContext,
  post::{GetPosts, GetPostsResponse},
  utils::check_private_instance_filtered,
};
use lemmy_db_schema::source::{community::Community, local_site::LocalSite};
use lemmy_db_views::{
  post_view::PostQuery,
  structs::{LocalUserView, PaginationCursor},
};
use lemmy_utils::error::{LemmyError, LemmyErrorExt, LemmyErrorType};

#[tracing::instrument(skip(context))]
pub async fn list_posts(
  data: Query<GetPosts>,
  context: Data<LemmyContext>,
  local_user_view: Option<LocalUserView>,
) -> Result<Json<GetPostsResponse>, LemmyError> {
  let local_site = LocalSite::read(&mut context.pool()).await?;

  // Todo(hexd0t): Check if the name resolution could be used to scrape private communities:
  let community_id = if let Some(name) = &data.community_name {
    Some(resolve_actor_identifier::<ApubCommunity, Community>(name, &context, &None, true).await?)
      .map(|c| c.id)
  } else {
    data.community_id
  };
  let filter = check_private_instance_filtered(
    &local_user_view,
    &local_site,
    &mut context.pool(),
    &community_id,
  )
  .await
  .map_err(|e| {
    tracing::warn!("Denying APub list_posts for {:?}", community_id);
    e
  })?;
  if filter {
    tracing::warn!("Filtering APub list_posts for {:?}", community_id);
    return Ok(Json(GetPostsResponse {
      posts: Vec::with_capacity(0),
      next_page: None,
    }));
  }

  let sort = data.sort;

  let page = data.page;
  let limit = data.limit;
  let saved_only = data.saved_only.unwrap_or_default();

  let liked_only = data.liked_only.unwrap_or_default();
  let disliked_only = data.disliked_only.unwrap_or_default();
  if liked_only && disliked_only {
    return Err(LemmyError::from(LemmyErrorType::ContradictingFilters));
  }

  let listing_type = Some(listing_type_with_default(
    data.type_,
    &local_site,
    community_id,
  )?);
  // parse pagination token
  let page_after = if let Some(pa) = &data.page_cursor {
    Some(pa.read(&mut context.pool()).await?)
  } else {
    None
  };

  let posts = PostQuery {
    local_user: local_user_view.as_ref(),
    listing_type,
    sort,
    community_id,
    saved_only,
    liked_only,
    disliked_only,
    page,
    page_after,
    limit,
    ..Default::default()
  }
  .list(&mut context.pool())
  .await
  .with_lemmy_type(LemmyErrorType::CouldntGetPosts)?;

  // if this page wasn't empty, then there is a next page after the last post on this page
  let next_page = posts.last().map(PaginationCursor::after_post);
  Ok(Json(GetPostsResponse { posts, next_page }))
}
