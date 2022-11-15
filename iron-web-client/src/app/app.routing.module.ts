import {RouterModule, Routes} from "@angular/router";
import {NgModule} from "@angular/core";
import {AppComponent} from "./app.component";
import {SplashscreenComponent} from "./splash-screen/splashscreen.component";

const routes: Routes = [
  {path: 'iron-gui', component: AppComponent},
  {path: 'session', loadChildren: () => import('./session/session.module').then(m => m.SessionModule)},
  {path: 'splashscreen', component: SplashscreenComponent},
  {path: '', redirectTo: 'iron-gui', pathMatch: 'full'}
];

@NgModule({
  imports: [RouterModule.forRoot(routes)],
  exports: [RouterModule]
})
export class AppRoutingModule {
}
