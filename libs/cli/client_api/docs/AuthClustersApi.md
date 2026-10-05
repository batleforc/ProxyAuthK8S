# \AuthClustersApi

All URIs are relative to *http://localhost*

Method | HTTP request | Description
------------- | ------------- | -------------
[**authorize**](AuthClustersApi.md#authorize) | **GET** /clusters/{ns}/{cluster}/oauth/authorize | Start the mediated OAuth Authorization Server flow for a cluster
[**callback**](AuthClustersApi.md#callback) | **GET** /clusters/{ns}/{cluster}/oauth/callback | Callback from the cluster's upstream OIDC provider, for the mediated OAuth Authorization Server flow
[**callback_login**](AuthClustersApi.md#callback_login) | **GET** /clusters/{ns}/{cluster}/auth/callback | Callback from the cluster's OIDC provider
[**cluster_login**](AuthClustersApi.md#cluster_login) | **GET** /clusters/{ns}/{cluster}/auth/login | Redirect to the cluster's login page
[**jwks**](AuthClustersApi.md#jwks) | **GET** /clusters/{ns}/{cluster}/oauth/jwks | The upstream identity provider's JSON Web Key Set, mirrored under the cluster's own path
[**oauth_authorization_server**](AuthClustersApi.md#oauth_authorization_server) | **GET** /clusters/{ns}/{cluster}/.well-known/oauth-authorization-server | OAuth 2.0 Authorization Server Metadata for the cluster's mediated OAuth Authorization Server
[**token**](AuthClustersApi.md#token) | **POST** /clusters/{ns}/{cluster}/oauth/token | Exchange a proxy-minted authorization code for the upstream tokens



## authorize

> authorize(ns, cluster, response_type, client_id, redirect_uri, code_challenge, state, scope, code_challenge_method)
Start the mediated OAuth Authorization Server flow for a cluster

Redirects the caller's browser to the cluster's upstream OIDC provider. If the cluster is not found, disabled, or discovery is not enabled, return 404.

### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**ns** | **String** | Namespace containing the cluster. | [required] |
**cluster** | **String** | Cluster name that should exist in the namespace. | [required] |
**response_type** | **String** | Must be `code`; this server only implements the authorization code grant. | [required] |
**client_id** | **String** | Unvalidated: the proxy mediates the whole flow, so external clients never need to register with the upstream provider. | [required] |
**redirect_uri** | **String** | Must be a loopback URI (`http://localhost` or `http://127.0.0.1`), any port/path. | [required] |
**code_challenge** | **String** | RFC 7636 PKCE code challenge (43-128 chars, unreserved charset). | [required] |
**state** | Option<**String**> | Opaque value echoed back verbatim on redirect. |  |
**scope** | Option<**String**> |  |  |
**code_challenge_method** | Option<**String**> | Must be `S256` when present; only S256 is supported. |  |

### Return type

 (empty response body)

### Authorization

No authorization required

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: Not defined

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## callback

> callback(ns, cluster, code, state)
Callback from the cluster's upstream OIDC provider, for the mediated OAuth Authorization Server flow

Not meant to be opened directly: the upstream provider redirects here after the caller authenticates. On success, redirects to the external client's own `redirect_uri` with a proxy-minted authorization code.

### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**ns** | **String** | Namespace containing the cluster. | [required] |
**cluster** | **String** | Cluster name that should exist in the namespace. | [required] |
**code** | **String** | Authorization code from the upstream OIDC provider. | [required] |
**state** | **String** | The correlation id this proxy generated at `/oauth/authorize`. | [required] |

### Return type

 (empty response body)

### Authorization

No authorization required

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: Not defined

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## callback_login

> models::CallbackModel callback_login(ns, cluster, x_front_callback, x_kubectl_callback, code, state)
Callback from the cluster's OIDC provider

If the cluster is not found or disabled, return 404.

### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**ns** | **String** | Namespace containing the cluster. | [required] |
**cluster** | **String** | Cluster name that should exist in the namespace. | [required] |
**x_front_callback** | Option<**String**> | If it's from the frontend, this header will be set. | [required] |
**x_kubectl_callback** | Option<**String**> | If it's from kubectl plugin, this header will be set. | [required] |
**code** | **String** | Authorization code from the OIDC provider. | [required] |
**state** | **String** | State parameter to prevent CSRF. | [required] |

### Return type

[**models::CallbackModel**](CallbackModel.md)

### Authorization

No authorization required

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## cluster_login

> String cluster_login(ns, cluster, x_front_callback, x_kubectl_callback)
Redirect to the cluster's login page

If the cluster is not found or disabled, return 404.

### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**ns** | **String** | Namespace containing the cluster. | [required] |
**cluster** | **String** | Cluster name that should exist in the namespace. | [required] |
**x_front_callback** | Option<**String**> | If it's from the frontend, this header will be set. | [required] |
**x_kubectl_callback** | Option<**String**> | If it's from kubectl plugin, this header will be set. | [required] |

### Return type

**String**

### Authorization

[bearer_auth](../README.md#bearer_auth)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: text/plain

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## jwks

> jwks(ns, cluster)
The upstream identity provider's JSON Web Key Set, mirrored under the cluster's own path

Stateless passthrough of the upstream provider's `jwks_uri`, so a caller verifying a token never needs to learn or contact the upstream provider's own hostname. If the cluster is not found, disabled, or does not have discovery enabled, return 404.

### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**ns** | **String** | Namespace containing the cluster. | [required] |
**cluster** | **String** | Cluster name that should exist in the namespace. | [required] |

### Return type

 (empty response body)

### Authorization

No authorization required

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: Not defined

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## oauth_authorization_server

> models::OAuthAuthorizationServerMetadata oauth_authorization_server(ns, cluster)
OAuth 2.0 Authorization Server Metadata for the cluster's mediated OAuth Authorization Server

Unauthenticated by nature (RFC 8414 discovery). If the cluster is not found, disabled, or does not have discovery enabled, return 404.

### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**ns** | **String** | Namespace containing the cluster. | [required] |
**cluster** | **String** | Cluster name that should exist in the namespace. | [required] |

### Return type

[**models::OAuthAuthorizationServerMetadata**](OAuthAuthorizationServerMetadata.md)

### Authorization

No authorization required

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## token

> models::TokenResponseBody token(ns, cluster, code, code_verifier, grant_type, redirect_uri)
Exchange a proxy-minted authorization code for the upstream tokens

RFC 6749 §4.1.3 token endpoint. If the cluster is not found, disabled, or discovery is not enabled, return 404.

### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**ns** | **String** | Namespace containing the cluster. | [required] |
**cluster** | **String** | Cluster name that should exist in the namespace. | [required] |
**code** | **String** | The proxy-minted code returned by `/oauth/callback`. | [required] |
**code_verifier** | **String** | RFC 7636 PKCE verifier for the `code_challenge` presented at `/oauth/authorize`. | [required] |
**grant_type** | **String** | Must be `authorization_code`; this server only implements that grant. | [required] |
**redirect_uri** | **String** | Must match the `redirect_uri` presented at `/oauth/authorize` (RFC 6749 §4.1.3). | [required] |

### Return type

[**models::TokenResponseBody**](TokenResponseBody.md)

### Authorization

No authorization required

### HTTP request headers

- **Content-Type**: application/x-www-form-urlencoded
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)

